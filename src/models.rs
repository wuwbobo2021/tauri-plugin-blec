use std::collections::HashMap;

use enumflags2::BitFlags;
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
            // NOTE: only one item. this is the limitation of `bluest` 0.6.x.
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Service {
    pub uuid: Uuid,
    pub characteristics: Vec<Characteristic>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Characteristic {
    pub uuid: Uuid,
    pub descriptors: Vec<Uuid>,
    pub properties: BitFlags<CharProps>,
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
            properties: get_flags(characteristic.properties().await?),
        })
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[enumflags2::bitflags]
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

fn get_flags(properties: bluest::CharacteristicProperties) -> BitFlags<CharProps, u8> {
    BitFlags::from_bits_truncate(properties.to_bits() as u8)
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
