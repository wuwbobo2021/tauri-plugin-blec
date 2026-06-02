use crate::error::Error;
use crate::event_handlers::{OnDisconnectHandler, SubscriptionHandler};
use crate::models::{self, AdapterState, BleDevice, ScanFilter, WriteType};
use crate::ALLOW_IBEACONS;
use bluest::{
    Adapter, AdvertisingDevice as DiscoveredDevice, Characteristic, ConnectionEvent, Device,
    DeviceId,
};

use futures::StreamExt;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio::time::{sleep, Instant};
use tracing::{debug, error, info, trace, warn};
use uuid::Uuid;

pub struct Handler {
    adapter: OnceLock<Arc<Adapter>>,
    known_devices: Arc<Mutex<HashMap<String, DiscoveredDevice>>>,
    scan_state: Mutex<ScanState>,
    connection: Mutex<Option<Connection>>,
    connected_rx: watch::Receiver<bool>,
    connected_tx: watch::Sender<bool>,
    connection_update_channels: Mutex<Vec<mpsc::Sender<bool>>>,
    /// Lock to serialize connection and disconnection operations
    connect_op_lock: Mutex<()>,
    /// Lock to serialize BLE GATT operations (only one read/write can be in flight at a time)
    gatt_op_lock: Mutex<()>,
}

struct ScanState {
    scan_task: Option<tokio::task::JoinHandle<()>>,
    scan_update_channel: Vec<mpsc::Sender<bool>>,
}

struct Connection {
    connected_dev: DiscoveredDevice,
    services: Vec<models::Service>,
    characs: Vec<Characteristic>,
    notify_listeners: Arc<Mutex<HashMap<Uuid, tokio::task::JoinHandle<()>>>>,
    device_event_handle: Option<tokio::task::JoinHandle<()>>,
    on_disconnect: OnDisconnectHandler,
}

impl Connection {
    pub(crate) async fn build(connected_dev: DiscoveredDevice) -> Result<Self, Error> {
        let mut conn = Self {
            connected_dev,
            services: Vec::new(),
            characs: Vec::new(),
            notify_listeners: Arc::new(Mutex::new(HashMap::new())),
            device_event_handle: None,
            on_disconnect: OnDisconnectHandler::None,
        };
        conn.refresh_services_characs().await?;
        Ok(conn)
    }

    /// Discover services and refresh `services` and `characs`, without GATT operation lock.
    async fn discover_services(&mut self) -> Result<(), Error> {
        debug!("starting service discovery");
        run_with_timeout(
            self.connected_dev.device.discover_services(),
            "discover services",
        )
        .await?;
        debug!("service discovery done");
        self.refresh_services_characs().await?;
        Ok(())
    }

    /// Refreshes `services` and `characs` with the known information from the
    /// underlying `bluest` API, without performing service discovery.
    async fn refresh_services_characs(&mut self) -> Result<(), Error> {
        let device = &self.connected_dev.device;
        let services = models::build_service_model(device).await?;
        let mut characs = vec![];
        for s in device.services().await? {
            for c in s.characteristics().await? {
                characs.push(c);
            }
        }
        self.services = services;
        self.characs = characs;
        Ok(())
    }

    fn get_charac(&self, uuid: Uuid, service: Option<Uuid>) -> Result<&Characteristic, Error> {
        if let Some(service) = service {
            info!("getting characteristic {uuid} from service {service}");
            let service = self
                .services
                .iter()
                .find(|s| s.uuid == service)
                .ok_or(Error::CharacNotAvailable(uuid.to_string()))?;
            service
                .characteristics
                .iter()
                .find(|c| c.uuid == uuid)
                .ok_or(Error::CharacNotAvailable(uuid.to_string()))?;
        } else {
            info!("getting characteristic {uuid}");
        }
        let charac = self.characs.iter().find(|c| c.uuid() == uuid);
        charac.ok_or(Error::CharacNotAvailable(uuid.to_string()))
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        for (_, handle) in self.notify_listeners.blocking_lock().drain() {
            handle.abort();
        }
        if let Some(handle) = self.device_event_handle.take() {
            handle.abort();
        }
    }
}

impl Handler {
    pub(crate) async fn new() -> Result<Self, Error> {
        let (connected_tx, connected_rx) = watch::channel(false);
        Ok(Self {
            adapter: OnceLock::new(),
            known_devices: Arc::new(Mutex::new(HashMap::new())),
            scan_state: Mutex::new(ScanState {
                scan_task: None,
                scan_update_channel: Vec::new(),
            }),
            connection: Mutex::new(None),
            connected_tx,
            connected_rx,
            connection_update_channels: Mutex::new(Vec::new()),
            connect_op_lock: Mutex::new(()),
            gatt_op_lock: Mutex::new(()),
        })
    }

    async fn get_or_init_adapter(&self) -> Result<Arc<Adapter>, Error> {
        if let Some(adapter) = self.adapter.get() {
            return Ok(adapter.clone());
        }
        let adapter = Adapter::default().await?;
        let arc_adapter = Arc::new(adapter);
        let _ = self.adapter.set(arc_adapter.clone());
        Ok(arc_adapter)
    }

    /// Returns true if a device is connected.
    pub fn is_connected(&self) -> bool {
        *self.connected_rx.borrow()
    }

    /// Returns true if the adapter is scanning.
    pub async fn is_scanning(&self) -> bool {
        if let Some(scan_task) = self.scan_state.lock().await.scan_task.as_ref() {
            !scan_task.is_finished()
        } else {
            false
        }
    }

    /// Gets the currently connected device.
    /// * Locks `connection` temporarily.
    async fn get_connected_device(&self) -> Option<Device> {
        self.connection
            .lock()
            .await
            .as_ref()
            .map(|conn| conn.connected_dev.device.clone())
    }

    /// Gets the currently connected device.
    /// * Locks `connection` temporarily.
    async fn require_connected_device(&self) -> Result<Device, Error> {
        self.get_connected_device()
            .await
            .ok_or(Error::NoDeviceConnected)
    }

    /// Gets the currently connected device's ID.
    /// * Locks `connection` temporarily.
    async fn connected_device_id(&self) -> Option<DeviceId> {
        self.connection
            .lock()
            .await
            .as_ref()
            .map(|conn| conn.connected_dev.device.id())
    }

    /// Finds a previously known device by `address`.
    /// * Locks `known_devices` temporarily.
    async fn lookup_device(&self, address: &str) -> Result<DiscoveredDevice, Error> {
        self.known_devices
            .lock()
            .await
            .get(address)
            .cloned()
            .ok_or_else(|| Error::UnknownPeripheral(address.to_string()))
    }

    /// Tries to get the characteristic with UUID `c` and service UUID `service`.
    /// Locks `connection` temporarily.
    async fn require_charac(
        &self,
        c: Uuid,
        service: Option<Uuid>,
    ) -> Result<Characteristic, Error> {
        let conn = self.connection.lock().await;
        conn.as_ref()
            .ok_or(Error::NoDeviceConnected)
            .and_then(|conn| Ok(conn.get_charac(c, service)?.clone()))
    }

    /// Takes a sender that will be used to send changes in the scanning status.
    ///
    /// # Example
    /// ```no_run
    /// use tauri::async_runtime;
    /// use tokio::sync::mpsc;
    /// async_runtime::block_on(async {
    ///     let handler = tauri_plugin_blec::get_handler().unwrap();
    ///     let (tx, mut rx) = mpsc::channel(1);
    ///     handler.set_scanning_update_channel(tx).await;
    ///     while let Some(scanning) = rx.recv().await {
    ///         println!("Scanning: {scanning}");
    ///     }
    /// });
    /// ```
    pub async fn set_scanning_update_channel(&self, tx: mpsc::Sender<bool>) {
        self.scan_state.lock().await.scan_update_channel.push(tx);
    }

    /// Takes a sender that will be used to send changes in the connection status
    /// # Example
    /// ```no_run
    /// use tauri::async_runtime;
    /// use tokio::sync::mpsc;
    /// async_runtime::block_on(async {
    ///     let handler = tauri_plugin_blec::get_handler().unwrap();
    ///     let (tx, mut rx) = mpsc::channel(1);
    ///     handler.set_connection_update_channel(tx).await;
    ///     while let Some(connected) = rx.recv().await {
    ///         println!("Connected: {connected}");
    ///     }
    /// });
    /// ```
    pub async fn set_connection_update_channel(&self, tx: mpsc::Sender<bool>) {
        self.connection_update_channels.lock().await.push(tx);
    }

    /// Connects to the device with the given address.
    ///
    /// If a callback is provided, it will be called when the device is disconnected.
    /// Because connecting sometimes fails especially on android, this method tries up to 3 times
    /// before returning an error.
    ///
    /// # Errors
    ///
    /// Returns an error if no devices are found, if the device is already connected,
    /// if the connection fails, or if the service/characteristics discovery fails.
    ///
    /// # Example
    /// ```no_run
    /// use tauri::async_runtime;
    /// use tauri_plugin_blec::OnDisconnectHandler;
    /// async_runtime::block_on(async {
    ///    let handler = tauri_plugin_blec::get_handler().unwrap();
    ///    handler.connect("00:00:00:00:00:00", OnDisconnectHandler::from_sync(|| println!("disconnected")), false).await.unwrap();
    /// });
    /// ```
    pub async fn connect(
        &self,
        address: &str,
        on_disconnect: OnDisconnectHandler,
        allow_ibeacons: bool,
    ) -> Result<(), Error> {
        if self.known_devices.lock().await.is_empty() {
            self.discover(None, 1000, ScanFilter::None, allow_ibeacons)
                .await?;
        }
        let _ = self.stop_scan().await; // cancel any running discovery

        // connect to the given address, try up to 3 times before returning an error

        let discovered = self.lookup_device(address).await?;
        let _conn_guard = self.connect_op_lock.lock().await;

        let device = discovered.device.clone();
        let conn_event_hdl =
            tokio::spawn(async move { Self::connection_event_handler(device).await });

        let mut connected = Ok(());
        for i in 0..3 {
            if let Err(e) = self.connect_internal(&discovered.device).await {
                if i < 2 {
                    warn!("Failed to connect device, retrying in 1s: {e}");
                    sleep(Duration::from_secs(1)).await;
                    continue;
                }
                connected = Err(e);
            } else {
                connected = Ok(());
                break;
            }
        }
        if let Err(e) = connected {
            let _ = self.connected_tx.send(false);
            error!("Failed to connect device: {e}");
            return Err(e);
        }

        let mut conn = Connection::build(discovered.clone()).await?;
        {
            let _gatt_guard = self.gatt_op_lock.lock().await;
            conn.discover_services().await?;
        }
        conn.on_disconnect = on_disconnect;
        conn.device_event_handle = Some(conn_event_hdl);
        self.connection.lock().await.replace(conn);
        self.send_connection_update(true).await;
        info!("connecting done");
        Ok(())
    }

    async fn connect_internal(&self, device: &Device) -> Result<(), Error> {
        trace!("connect_device: initiating connection to {}", device.id());
        debug!("connecting to {}", device.id());
        let mut connected_rx = self.connected_rx.clone();
        {
            if device.is_connected().await {
                debug!("Device already connected");
                self.connected_tx
                    .send(true)
                    .expect("failed to send connected update");
                return Ok(());
            }
        }
        debug!("Connecting to device");
        {
            let adapter = self.get_or_init_adapter().await?;
            let _gatt_guard = self.gatt_op_lock.lock().await;
            run_with_timeout(adapter.connect_device(device), "Connect").await?;
        }
        // wait for the actual connection to be established
        if !*connected_rx.borrow_and_update() {
            info!("waiting for connection event");
            connected_rx
                .changed()
                .await
                .expect("failed to wait for connection event");
        }
        if !*self.connected_rx.borrow() {
            // still not connected
            warn!("Still not connected after connection event");
            return Err(Error::ConnectionFailed);
        }
        trace!("connect_device: connection established to {}", device.id());
        info!("device connected");
        Ok(())
    }

    /// Disconnects from the connected device.
    /// This triggers a disconnect and then waits for the actual disconnect event from the adapter.
    /// # Errors
    /// Returns an error if no device is connected or if the disconnect fails.
    /// # Panics
    /// Panics if there is an error with handling the internal disconnect event.
    pub async fn disconnect(&self) -> Result<(), Error> {
        trace!("disconnect: user-initiated disconnect");
        info!("disconnect triggered by user");
        let mut connected_rx = self.connected_rx.clone();

        let dev = self.require_connected_device().await?;
        let _conn_guard = self.connect_op_lock.lock().await;

        if dev.is_connected().await {
            assert!(
                (*connected_rx.borrow_and_update()),
                "connected_rx is false with a device being connected, this is a bug"
            );
            let adapter = self.get_or_init_adapter().await?;
            adapter.disconnect_device(&dev).await?;
        } else {
            debug!("device is not connected");
            return Err(Error::NoDeviceConnected);
        }

        // the change will be triggered by handle_event -> handle_disconnect which runs in another
        // task
        connected_rx
            .changed()
            .await
            .expect("failed to wait for disconnect event");
        if *self.connected_rx.borrow() {
            // still connected
            return Err(Error::DisconnectFailed);
        }
        Ok(())
    }

    async fn connection_event_handler(device: Device) {
        let handler = crate::get_handler().unwrap();
        let adapter = handler.get_or_init_adapter().await.unwrap();
        let mut conn_events = match adapter.device_connection_events(&device).await {
            Ok(stream) => stream,
            Err(e) => {
                error!("`Adapter::device_connection_events` failed in `device_event_handle`: {e}");
                return;
            }
        };
        while let Some(ev) = conn_events.next().await {
            match ev {
                ConnectionEvent::Connected => {
                    handler.handle_connect(device.id()).await;
                }
                ConnectionEvent::Disconnected => {
                    let _ = handler.handle_disconnect(device.id()).await;
                }
            }
        }
    }

    #[allow(clippy::redundant_closure_for_method_calls)]
    async fn handle_connect(&self, peripheral_id: DeviceId) {
        if let Some(connected_device) = self.connected_device_id().await {
            if connected_device == peripheral_id {
                trace!("handle_connect: DeviceConnected event for {peripheral_id}");
                debug!("connection to {peripheral_id} established");
                self.connected_tx
                    .send(true)
                    .expect("failed to send connected update");
                debug!("connected_tx updated");
            } else {
                // event not for currently connected device, ignore
                warn!("Unexpected connect event for device {peripheral_id}, connected device is {connected_device}");
            }
        } else {
            warn!(
                "connect event for device {peripheral_id} received without waiting for connection"
            );
        }
    }

    /// Clears internal state, updates connected flag and calls disconnect callback
    async fn handle_disconnect(&self, peripheral_id: DeviceId) -> Result<(), Error> {
        trace!("handle_disconnect: DeviceDisconnected event for {peripheral_id}");
        info!("Handling disconnect for {peripheral_id}");
        let connected = self.connected_device_id().await;
        if connected.as_ref().is_none_or(|c| *c != peripheral_id) {
            // event not for currently connected device, ignore
            warn!("Unexpected disconnect event for device {peripheral_id}, connected device is {connected:?}",);
            return Ok(());
        }
        {
            info!("disconnecting");
            let conn = self.connection.lock().await.take();
            if let Some(mut conn) = conn {
                conn.on_disconnect.take().run().await;
            }
        }
        self.send_connection_update(false).await;
        self.connected_tx
            .send(false)
            .expect("failed to send connected update");
        Ok(())
    }

    // XXX: with `bluest`, new devices are always discovered one by one,
    // there could be `Sender<BleDevice>` instead of `Sender<Vec<BleDevice>>`.
    /// Scans for `timeout` milliseconds and periodically sends discovered devices
    /// to the given channel.
    ///
    /// A task is spawned to handle the scan and send the devices, so the function
    /// returns immediately.
    ///
    /// A Variant of [`ScanFilter`] can be provided to filter the discovered devices
    /// When `allow_ibeacons` is set to true, android will request fine location permission to
    /// allow finding and connecting to iBeacons.
    ///
    /// # Errors
    /// Returns an error if starting the scan fails
    /// # Panics
    /// Panics if there is an error getting devices from the adapter
    /// # Example
    /// ```no_run
    /// use tauri::async_runtime;
    /// use tokio::sync::mpsc;
    /// use tauri_plugin_blec::models::ScanFilter;
    ///
    /// async_runtime::block_on(async {
    ///     let handler = tauri_plugin_blec::get_handler().unwrap();
    ///     let (tx, mut rx) = mpsc::channel(1);
    ///     handler.discover(Some(tx),1000, ScanFilter::None, false).await.unwrap();
    ///     while let Some(devices) = rx.recv().await {
    ///         println!("Discovered {devices:?}");
    ///     }
    /// });
    /// ```
    pub async fn discover(
        &self,
        tx: Option<mpsc::Sender<Vec<BleDevice>>>,
        timeout: u64,
        filter: ScanFilter,
        allow_ibeacons: bool,
    ) -> Result<(), Error> {
        if let ScanFilter::ManufacturerDataMasked(_, ref data, ref mask) = filter {
            if data.len() != mask.len() {
                return Err(Error::InvalidFilterMask);
            }
        }
        self.stop_scan().await?;
        // start a new scan
        ALLOW_IBEACONS.store(allow_ibeacons, std::sync::atomic::Ordering::Release);
        let adapter = self.get_or_init_adapter().await?;
        let self_devices = self.known_devices.clone();
        let (tx_init, rx_init) = oneshot::channel();
        let mut state = self.scan_state.lock().await;
        state.scan_task.replace(tokio::task::spawn(async move {
            Self::discover_handler(adapter, self_devices, tx_init, tx, timeout, filter).await
        }));
        rx_init.await.unwrap()
    }

    async fn discover_handler(
        adapter: Arc<Adapter>,
        self_devices: Arc<Mutex<HashMap<String, DiscoveredDevice>>>,
        tx_init_signal: oneshot::Sender<Result<(), Error>>,
        tx: Option<mpsc::Sender<Vec<BleDevice>>>,
        timeout: u64,
        filter: ScanFilter,
    ) {
        let handler = crate::get_handler().unwrap();
        let mut scan_stream = match adapter.scan(&[]).await {
            Ok(stream) => {
                let _ = tx_init_signal.send(Ok(()));
                stream
            }
            Err(e) => {
                error!("`Adapter::scan` failed: {e}");
                let _ = tx_init_signal.send(Err(e.into()));
                return;
            }
        };
        self_devices.lock().await.clear();
        handler.send_scan_update(true).await;
        let t_timeout = Instant::now() + Duration::from_millis(timeout);
        loop {
            let scan_next = async {
                scan_stream
                    .next()
                    .await
                    .ok_or(Error::Timeout("scan timeout".into()))
            };
            let timeout = t_timeout
                .checked_duration_since(Instant::now())
                .unwrap_or(Duration::from_secs(0));
            let Ok(discovered) = run_with_given_timeout(scan_next, timeout, "scan").await else {
                break;
            };
            let Ok(ble_device) = BleDevice::from_bluest(&discovered).await else {
                error!("`BleDevice::from_bluest` failed in scan process");
                continue;
            };
            let id = discovered.device.id().to_string();
            if filter_device(&discovered, &filter) {
                // refreshes advertisement data even if the device with `id` is previously known.
                self_devices.lock().await.insert(id, discovered);
            }
            if let Some(tx) = &tx {
                tx.send(vec![ble_device])
                    .await
                    .expect("failed to send devices");
            }
        }
        // the scanning is stopped in the drop glue of the `bluest` stream implementation.
        drop(scan_stream);
        handler.send_scan_update(false).await;
        info!("`discover_handler` ended");
    }

    /// Discover provided services and charecteristics.
    ///
    /// If the device with `address` was already connected, it will stay connected.
    ///
    /// If the device with `address` is not connected, a connection is made in order to discover
    /// the services and characteristics. If some other device was previously connected, it is
    /// disconnected. After the service discovery is done, the device with `address` is disconnected.
    ///
    ///
    /// # Errors
    /// Returns an error if the device is not found, if the connection fails, or if the discovery fails.
    ///
    /// # Panics
    /// Panics if there is an error with the internal disconnect event.
    pub async fn discover_services(&self, address: &str) -> Result<Vec<models::Service>, Error> {
        let prev_connected_dev = self.get_connected_device().await;
        if let Some(device) = prev_connected_dev.as_ref() {
            if address == device.id().to_string() {
                // XXX: should service discovery be performed again?
                return models::build_service_model(device).await;
            }
        }
        let device = self.lookup_device(address).await?;
        if let Err(e) = self.connect(address, OnDisconnectHandler::None, true).await {
            error!("Failed to connect for service discovery: {e}");
            return Err(e);
        }
        let result = models::build_service_model(&device.device).await;
        if let Err(e) = self.disconnect().await {
            error!("Failed to disconnect after service discovery: {e}");
        }
        result
    }

    /// Stops scanning for devices.
    ///
    /// # Errors
    /// Stops an ongoing scan. The polling task is aborted first, then the
    /// adapter scan is stopped (best-effort — it may have already been
    /// stopped by the polling task finishing).
    pub async fn stop_scan(&self) -> Result<(), Error> {
        if let Some(handle) = self.scan_state.lock().await.scan_task.take() {
            handle.abort();
            self.send_scan_update(false).await;
        }
        Ok(())
    }

    /// Sends data to the given characteristic of the connected device.
    ///
    /// # Errors
    /// Returns an error if no device is connected or the characteristic is not available
    /// or if the write operation fails.
    ///
    /// # Example
    /// ```no_run
    /// use tauri::async_runtime;
    /// use uuid::{Uuid,uuid};
    /// use tauri_plugin_blec::models::WriteType;
    ///
    /// const CHARACTERISTIC_UUID: Uuid = uuid!("51FF12BB-3ED8-46E5-B4F9-D64E2FEC021B");
    /// async_runtime::block_on(async {
    ///     let handler = tauri_plugin_blec::get_handler().unwrap();
    ///     let data = [1,2,3,4,5];
    ///     let response = handler.send_data(CHARACTERISTIC_UUID, None, &data, WriteType::WithResponse).await.unwrap();
    /// });
    /// ```
    pub async fn send_data(
        &self,
        c: Uuid,
        service: Option<Uuid>,
        data: &[u8],
        write_type: models::WriteType,
    ) -> Result<(), Error> {
        let _gatt_guard = self.gatt_op_lock.lock().await;
        let charac = self.require_charac(c, service).await?;
        trace!(
            "sending {} bytes to characteristic {c}: {:02x?}",
            data.len(),
            data
        );
        run_with_timeout(
            async {
                match write_type {
                    WriteType::WithResponse => charac.write(data).await,
                    WriteType::WithoutResponse => charac.write_without_response(data).await,
                }
            },
            "write",
        )
        .await?;
        Ok(())
    }

    /// Receives data from the given characteristic of the connected device
    /// Returns the data as a vector of bytes
    /// # Errors
    /// Returns an error if no device is connected or the characteristic is not available
    /// or if the read operation fails
    /// # Example
    /// ```no_run
    /// use tauri::async_runtime;
    /// use uuid::{Uuid,uuid};
    /// const CHARACTERISTIC_UUID: Uuid = uuid!("51FF12BB-3ED8-46E5-B4F9-D64E2FEC021B");
    /// async_runtime::block_on(async {
    ///     let handler = tauri_plugin_blec::get_handler().unwrap();
    ///     let response = handler.recv_data(CHARACTERISTIC_UUID, None).await.unwrap();
    /// });
    /// ```
    pub async fn recv_data(&self, c: Uuid, service: Option<Uuid>) -> Result<Vec<u8>, Error> {
        let _gatt_guard = self.gatt_op_lock.lock().await;
        let charac = self.require_charac(c, service).await?;
        let data = run_with_timeout(charac.read(), "read").await?;
        trace!(
            "received {} bytes from characteristic {c}: {:02x?}",
            data.len(),
            data
        );
        Ok(data)
    }

    /// Subscribe to notifications from the given characteristic
    /// The callback will be called whenever a notification is received
    /// # Errors
    /// Returns an error if no device is connected or the characteristic is not available
    /// or if the subscribe operation fails
    /// # Example
    /// ```no_run
    /// use tauri::async_runtime;
    /// use uuid::{Uuid,uuid};
    /// const CHARACTERISTIC_UUID: Uuid = uuid!("51FF12BB-3ED8-46E5-B4F9-D64E2FEC021B");
    /// async_runtime::block_on(async {
    ///     let handler = tauri_plugin_blec::get_handler().unwrap();
    ///     let response = handler.subscribe(CHARACTERISTIC_UUID, None, |data| println!("received {data:?}")).await.unwrap();
    /// });
    /// ```
    pub async fn subscribe(
        &self,
        c: Uuid,
        service: Option<Uuid>,
        callback: impl Into<SubscriptionHandler> + Send + 'static,
    ) -> Result<(), Error> {
        let charac = self.require_charac(c, service).await?;
        let char_id = charac.uuid();
        let (tx_init, rx_init) = oneshot::channel();
        let listen_handle =
            tokio::task::spawn(
                async move { Self::subscribe_handler(charac, tx_init, callback).await },
            );
        rx_init.await.unwrap()?;
        let conn_guard = self.connection.lock().await;
        if let Some(conn) = conn_guard.as_ref() {
            conn.notify_listeners
                .lock()
                .await
                .insert(char_id, listen_handle);
            Ok(())
        } else {
            Err(Error::NoDeviceConnected)
        }
    }

    async fn subscribe_handler(
        charac: Characteristic,
        tx_init_signal: oneshot::Sender<Result<(), Error>>,
        callback: impl Into<SubscriptionHandler> + Send + 'static,
    ) {
        let id = charac.uuid();
        let notify_stream = {
            let handler = crate::get_handler().unwrap();
            let _gatt_guard = handler.gatt_op_lock.lock().await;
            info!("subscribing to characteristic {charac:?}");
            run_with_timeout(charac.notify(), "subscribe").await
        };
        info!("subscribed successfully");
        let mut notify_stream = match notify_stream {
            Ok(stream) => {
                let _ = tx_init_signal.send(Ok(()));
                stream
            }
            Err(e) => {
                error!("failed to create notify stream in subscribe listener: {e}");
                let _ = tx_init_signal.send(Err(e));
                return;
            }
        };
        let callback = callback.into();
        while let Some(next_result) = notify_stream.next().await {
            if let Ok(data) = next_result {
                trace!(
                    "notification from {}: {} bytes: {:02x?}",
                    id,
                    data.len(),
                    data
                );
                // run callback
                trace!("starting callback for {:?}", id);
                callback.run(data).await;
                trace!("callback for {:?} finished", id);
            }
        }
        info!("Notification stream of {id:?} ended");
    }

    /// Unsubscribe from notifications for the given characteristic.
    /// This will also remove the callback from the list of listeners.
    ///
    /// # Errors
    /// Returns an error if no device is connected or if the unsubscribe operation fails.
    pub async fn unsubscribe(&self, c: Uuid) -> Result<(), Error> {
        let conn_guard = self.connection.lock().await;
        if let Some(conn) = conn_guard.as_ref() {
            let _ = conn.notify_listeners.lock().await.remove(&c);
            Ok(())
        } else {
            Err(Error::NoDeviceConnected)
        }
    }

    /// Returns the connected device.
    ///
    /// # Errors
    /// Returns an error if no device is connected
    pub async fn connected_device(&self) -> Result<BleDevice, Error> {
        let connected_dev = {
            let conn_guard = self.connection.lock().await;
            let conn = conn_guard.as_ref().ok_or(Error::NoDeviceConnected)?;
            conn.connected_dev.clone()
        };
        BleDevice::from_bluest(&connected_dev).await
    }

    async fn send_connection_update(&self, state: bool) {
        let tx = &mut &mut self.connection_update_channels.lock().await;
        info!("sending connection update to {} listeners", tx.len());
        let mut remove = vec![];
        for (i, t) in tx.iter_mut().enumerate() {
            if let Err(e) = t.send(state).await {
                warn!("Failed to send connection update: {e}");
                remove.push(i);
            }
        }
    }

    async fn send_scan_update(&self, state: bool) {
        let tx = &mut &mut self.scan_state.lock().await.scan_update_channel;
        let mut remove = vec![];
        for (i, t) in tx.iter_mut().enumerate() {
            if let Err(e) = t.send(state).await {
                warn!("Failed to send scan update: {e}");
                remove.push(i);
            }
        }
    }

    pub async fn get_adapter_state(&self) -> AdapterState {
        let adapter = match self.get_or_init_adapter().await {
            Ok(a) => a,
            Err(e) => {
                error!("Failed to init adapter for state check: {e}");
                return AdapterState::Unknown;
            }
        };

        match adapter.is_available().await {
            Ok(state) => match state {
                true => AdapterState::On,
                false => AdapterState::Off,
            },
            Err(e) => {
                error!("Failed to get adapter state: {e}");
                AdapterState::Unknown
            }
        }
    }
}

async fn run_with_timeout<T: Send>(
    fut: impl Future<Output = Result<T, bluest::Error>> + Send,
    cmd: &str,
) -> Result<T, Error> {
    run_with_given_timeout(
        async { fut.await.map_err(Error::Btleplug) },
        Duration::from_secs(5),
        cmd,
    )
    .await
}

async fn run_with_given_timeout<T: Send>(
    fut: impl Future<Output = Result<T, Error>> + Send,
    timeout: Duration,
    cmd: &str,
) -> Result<T, Error> {
    tokio::time::timeout(timeout, fut)
        .await
        .map_err(|_| Error::Timeout(cmd.to_string()))?
}

fn filter_device(discovered: &DiscoveredDevice, filter: &ScanFilter) -> bool {
    match filter {
        ScanFilter::None => true,
        ScanFilter::Service(uuid) => discovered.adv_data.services.iter().any(|s| s == uuid),
        ScanFilter::AnyService(uuids) => discovered
            .adv_data
            .services
            .iter()
            .any(|s| uuids.contains(s)),
        ScanFilter::AllServices(uuids) => discovered
            .adv_data
            .services
            .iter()
            .all(|s| uuids.contains(s)),
        ScanFilter::ManufacturerData(key, value) => discovered
            .adv_data
            .manufacturer_data
            .as_ref()
            .is_some_and(|v| v.company_id == *key && &v.data == value),
        ScanFilter::ManufacturerDataMasked(key, value, maks) => discovered
            .adv_data
            .manufacturer_data
            .as_ref()
            .is_some_and(|v| {
                v.company_id == *key
                    && v.data
                        .iter()
                        .zip(maks.iter())
                        .zip(value.iter())
                        .all(|((d, m), v)| (d & m) == (*v & m))
            }),
    }
}
