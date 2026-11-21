//! Leitura da bateria dos dispositivos Bluetooth.
//!
//! Fonte 1: propriedade PnP que o próprio Windows preenche (a mesma que a tela
//! de Configurações mostra). Barata, não abre conexão com o aparelho.
//! Fonte 2 (fallback): GATT Battery Service, só para dispositivos BLE conectados
//! que não informaram bateria pela fonte 1.

use std::collections::HashMap;

use windows::core::{GUID, PCWSTR};
use windows::Win32::Devices::Properties::DEVPROPKEY;
use windows::Win32::Devices::DeviceAndDriverInstallation::*;
use windows::Win32::Devices::Properties::*;

#[derive(Clone, Debug)]
pub struct Device {
    /// Endereço Bluetooth em hex (chave estável).
    pub key: String,
    pub name: String,
    pub level: u8,
}

/// DEVPKEY_Bluetooth_Battery
const BATTERY_KEY: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5),
    pid: 2,
};

/// Bit DN_DEVICE_DISCONNECTED do status do devnode.
const DN_DEVICE_DISCONNECTED: u32 = 0x0200_0000;

pub fn scan() -> Vec<Device> {
    // Só em debug: MONITORBT_FAKE="Fone:12,Mouse:88" simula dispositivos.
    #[cfg(debug_assertions)]
    if let Ok(fake) = std::env::var("MONITORBT_FAKE") {
        return fake
            .split(',')
            .filter_map(|s| s.split_once(':'))
            .map(|(n, l)| Device { key: format!("FAKE{n}"), name: n.into(), level: l.parse().unwrap_or(0) })
            .collect();
    }

    let mut names: HashMap<String, String> = HashMap::new();
    let mut levels: HashMap<String, (u8, String)> = HashMap::new();
    let mut ble_candidates: Vec<String> = Vec::new();

    unsafe {
        let Ok(set) = SetupDiGetClassDevsW(None, PCWSTR::null(), None, DIGCF_ALLCLASSES | DIGCF_PRESENT)
        else {
            return Vec::new();
        };

        let mut i = 0u32;
        loop {
            let mut info = SP_DEVINFO_DATA {
                cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            if SetupDiEnumDeviceInfo(set, i, &mut info).is_err() {
                break;
            }
            i += 1;

            let Some(id) = instance_id(set, &info) else { continue };
            let Some(addr) = parse_addr(&id) else { continue };
            let connected = is_connected(info.DevInst);

            let is_dev_node = id.starts_with("BTHENUM\\DEV_") || id.starts_with("BTHLE\\DEV_");
            if is_dev_node {
                if let Some(n) = string_prop(set, &info, &DEVPKEY_Device_FriendlyName) {
                    names.insert(addr.clone(), n);
                }
                if id.starts_with("BTHLE\\DEV_") && connected {
                    ble_candidates.push(addr.clone());
                }
            }

            if !connected {
                continue;
            }
            if let Some(level) = byte_prop(set, &info, &BATTERY_KEY) {
                let fallback = string_prop(set, &info, &DEVPKEY_Device_FriendlyName).unwrap_or_default();
                levels.entry(addr).or_insert((level.min(100), fallback));
            }
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
    }

    // Fallback GATT só para BLE conectados sem bateria via PnP.
    ble_candidates.retain(|a| !levels.contains_key(a));
    if !ble_candidates.is_empty() {
        for addr in ble_candidates {
            if let Some(level) = gatt::read_level(&addr) {
                levels.insert(addr, (level.min(100), String::new()));
            }
        }
    }

    let mut out: Vec<Device> = levels
        .into_iter()
        .map(|(key, (level, fallback))| {
            let name = names
                .get(&key)
                .cloned()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| if fallback.is_empty() { key.clone() } else { fallback });
            Device { key, name, level }
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Extrai o endereço Bluetooth (12 dígitos hex) do instance id.
fn parse_addr(id: &str) -> Option<String> {
    id.split(['\\', '_', '&', '#'])
        .filter(|t| t.len() == 12 && t.bytes().all(|b| b.is_ascii_hexdigit()))
        .last()
        .map(|s| s.to_ascii_uppercase())
}

unsafe fn is_connected(devinst: u32) -> bool {
    let mut status = CM_DEVNODE_STATUS_FLAGS(0);
    let mut problem = CM_PROB(0);
    if CM_Get_DevNode_Status(&mut status, &mut problem, devinst, 0) != CR_SUCCESS {
        return false;
    }
    status.0 & DN_DEVICE_DISCONNECTED == 0
}

unsafe fn instance_id(set: HDEVINFO, info: &SP_DEVINFO_DATA) -> Option<String> {
    let mut buf = [0u16; 512];
    SetupDiGetDeviceInstanceIdW(set, info, Some(&mut buf), None).ok()?;
    Some(from_wide(&buf).to_ascii_uppercase())
}

unsafe fn raw_prop(set: HDEVINFO, info: &SP_DEVINFO_DATA, key: &DEVPROPKEY, buf: &mut [u8]) -> Option<(DEVPROPTYPE, usize)> {
    let mut ty = DEVPROPTYPE(0);
    let mut size = 0u32;
    SetupDiGetDevicePropertyW(set, info, key, &mut ty, Some(buf), Some(&mut size), 0).ok()?;
    Some((ty, size as usize))
}

unsafe fn byte_prop(set: HDEVINFO, info: &SP_DEVINFO_DATA, key: &DEVPROPKEY) -> Option<u8> {
    let mut buf = [0u8; 8];
    let (ty, size) = raw_prop(set, info, key, &mut buf)?;
    (ty == DEVPROP_TYPE_BYTE && size >= 1).then(|| buf[0])
}

unsafe fn string_prop(set: HDEVINFO, info: &SP_DEVINFO_DATA, key: &DEVPROPKEY) -> Option<String> {
    let mut buf = [0u8; 512];
    let (ty, size) = raw_prop(set, info, key, &mut buf)?;
    if ty != DEVPROP_TYPE_STRING {
        return None;
    }
    let wide: Vec<u16> = buf[..size].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some(from_wide(&wide))
}

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

mod gatt {
    use windows::Devices::Bluetooth::GenericAttributeProfile::*;
    use windows::Devices::Bluetooth::*;
    use windows::Storage::Streams::DataReader;

    pub fn read_level(addr_hex: &str) -> Option<u8> {
        let addr = u64::from_str_radix(addr_hex, 16).ok()?;
        let dev = BluetoothLEDevice::FromBluetoothAddressAsync(addr).ok()?.get().ok()?;
        let result = read(&dev);
        let _ = dev.Close();
        result
    }

    fn read(dev: &BluetoothLEDevice) -> Option<u8> {
        if dev.ConnectionStatus().ok()? != BluetoothConnectionStatus::Connected {
            return None;
        }
        let services = dev
            .GetGattServicesForUuidWithCacheModeAsync(GattServiceUuids::Battery().ok()?, BluetoothCacheMode::Cached)
            .ok()?
            .get()
            .ok()?;
        if services.Status().ok()? != GattCommunicationStatus::Success {
            return None;
        }
        let service = services.Services().ok()?.GetAt(0).ok()?;
        let level = (|| {
            let chars = service
                .GetCharacteristicsForUuidAsync(GattCharacteristicUuids::BatteryLevel().ok()?)
                .ok()?
                .get()
                .ok()?;
            let ch = chars.Characteristics().ok()?.GetAt(0).ok()?;
            let value = ch.ReadValueWithCacheModeAsync(BluetoothCacheMode::Uncached).ok()?.get().ok()?;
            if value.Status().ok()? != GattCommunicationStatus::Success {
                return None;
            }
            DataReader::FromBuffer(&value.Value().ok()?).ok()?.ReadByte().ok()
        })();
        let _ = service.Close();
        level
    }
}

#[cfg(test)]
mod tests {
    /// `cargo test scan -- --ignored --nocapture` lista o que o app enxerga agora.
    #[test]
    #[ignore]
    fn scan_now() {
        let t = std::time::Instant::now();
        let devs = super::scan();
        println!("{} dispositivo(s) em {:?}", devs.len(), t.elapsed());
        for d in devs {
            println!("  {} [{}] {}%", d.name, d.key, d.level);
        }
    }
}
