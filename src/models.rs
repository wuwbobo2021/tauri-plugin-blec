use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BleDevice {
    pub address: String,
    pub name: String,
    pub is_connected: bool,
    pub is_bonded: bool,
    pub manufacturer_data: HashMap<u16, Vec<u8>>,
    pub service_data: HashMap<Uuid, Vec<u8>>,
    pub services: Vec<Uuid>,
    pub rssi: Option<i16>,
    pub tx_power_level: Option<i16>,
}

impl Eq for BleDevice {}

impl PartialOrd for BleDevice {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BleDevice {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.address.cmp(&other.address)
    }
}

impl PartialEq for BleDevice {
    fn eq(&self, other: &Self) -> bool {
        self.address == other.address
    }
}

impl BleDevice {
    pub(crate) async fn from_bluest(
        adv_dev: &bluest::AdvertisingDevice,
    ) -> Result<Self, error::Error> {
        let adv_data = adv_dev.adv_data.clone();
        let address = adv_dev.device.id().to_string();
        let name = adv_dev
            .adv_data
            .local_name
            .clone()
            .unwrap_or_else(|| adv_dev.device.id().to_string());
        let mut manufacturer_data = HashMap::new();
        if let Some(man_data) = adv_data.manufacturer_data {
            manufacturer_data.insert(man_data.company_id, man_data.data);
        }
        let is_connected = adv_dev.device.is_connected().await;
        let rssi = if is_connected {
            adv_dev.device.rssi().await.ok()
        } else {
            adv_dev.rssi
        };
        let is_bonded = adv_dev.device.is_paired().await?;
        Ok(Self {
            address,
            name,
            manufacturer_data,
            service_data: adv_data.service_data,
            services: adv_data.services,
            rssi,
            is_connected,
            is_bonded,
            tx_power_level: adv_data.tx_power_level,
        })
    }
}

pub async fn build_service_model(device: &bluest::Device) -> Result<Vec<Service>, error::Error> {
    let mut service_models = Vec::new();
    for service in device.services().await? {
        let service = Service::from_bluest(&service).await?;
        service_models.push(service);
    }
    Ok(service_models)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Service {
    pub uuid: Uuid,
    pub characteristics: Vec<Characteristic>,
}

impl Service {
    pub(crate) async fn from_bluest(service: &bluest::Service) -> Result<Self, error::Error> {
        let mut characteristics = Vec::new();
        for char in service.characteristics().await? {
            characteristics.push(Characteristic::from_bluest(&char).await?);
        }
        Ok(Self {
            uuid: service.uuid(),
            characteristics,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Characteristic {
    pub uuid: Uuid,
    pub descriptors: Vec<Uuid>,
    pub properties: CharProps,
}

impl Characteristic {
    pub(crate) async fn from_bluest(
        characteristic: &bluest::Characteristic,
    ) -> Result<Self, error::Error> {
        Ok(Self {
            uuid: characteristic.uuid(),
            descriptors: characteristic
                .descriptors()
                .await?
                .iter()
                .map(|d| d.uuid())
                .collect(),
            properties: characteristic.properties().await?.into(),
        })
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[repr(u8)]
pub enum CharProps {
    Broadcast,
    Read,
    WriteWithoutResponse,
    Write,
    Notify,
    Indicate,
    AuthenticatedSignedWrites,
    ExtendedProperties,
}

impl From<bluest::CharacteristicProperties> for CharProps {
    fn from(flag: bluest::CharacteristicProperties) -> Self {
        if flag.broadcast {
            CharProps::Broadcast
        } else if flag.read {
            CharProps::Read
        } else if flag.write_without_response {
            CharProps::WriteWithoutResponse
        } else if flag.write {
            CharProps::Write
        } else if flag.notify {
            CharProps::Notify
        } else if flag.indicate {
            CharProps::Indicate
        } else if flag.authenticated_signed_writes {
            CharProps::AuthenticatedSignedWrites
        } else if flag.extended_properties {
            CharProps::ExtendedProperties
        } else {
            unreachable!()
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WriteType {
    /// aka request.
    WithResponse,
    /// aka command.
    WithoutResponse,
}

/// Filter for discovering devices.
/// Only devices matching the filter will be returned by the `handler::discover` method
pub enum ScanFilter {
    None,
    /// Matches if the device advertises the specified service.
    Service(Uuid),
    /// Matches if the device advertises any of the specified services.
    AnyService(Vec<Uuid>),
    /// Matches if the device advertises all of the specified services.
    AllServices(Vec<Uuid>),
    /// Matches if the device advertises the specified manufacturer data.
    ManufacturerData(u16, Vec<u8>),
    /// Matches if the device advertises the specified manufacturer data, checking only the bits
    /// that are 1 in the mask
    ManufacturerDataMasked(u16, Vec<u8>, Vec<u8>),
}

/// State of the Bluetooth adapter
#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum AdapterState {
    Unknown,
    Off,
    On,
}
