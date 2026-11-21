//! Liga/desliga o início com o Windows (HKCU\...\Run).

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::System::Registry::*;

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE: PCWSTR = w!("MonitorBT");

pub fn is_enabled() -> bool {
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE, RRF_RT_REG_SZ, None, None, None).is_ok() }
}

pub fn set(enabled: bool) {
    unsafe {
        if enabled {
            let Ok(exe) = std::env::current_exe() else { return };
            let cmd = format!("\"{}\"\0", exe.display());
            let wide: Vec<u16> = cmd.encode_utf16().collect();
            let bytes = std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2);
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                VALUE,
                REG_SZ.0,
                Some(bytes.as_ptr() as *const _),
                bytes.len() as u32,
            );
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE);
        }
    }
}

/// true se a barra de tarefas usa tema claro.
pub fn light_theme() -> bool {
    let mut data = 0u32;
    let mut size = 4u32;
    let key = HSTRING::from("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &key,
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut u32 as *mut _),
            Some(&mut size),
        )
        .is_ok()
    };
    ok && data == 1
}

/// Windows 11 joga ícones novos para o menu de excesso (^). Marca os ícones deste exe
/// como visíveis na primeira vez que aparecem (só se o usuário ainda não escolheu).
pub fn promote_tray_icons() {
    let Ok(exe) = std::env::current_exe() else { return };
    let exe = exe.to_string_lossy().to_lowercase();
    unsafe {
        let mut root = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, w!("Control Panel\\NotifyIconSettings"), 0, KEY_READ | KEY_SET_VALUE, &mut root)
            .is_err()
        {
            return;
        }
        let mut i = 0u32;
        loop {
            let mut name = [0u16; 64];
            let mut len = name.len() as u32;
            if RegEnumKeyExW(root, i, windows::core::PWSTR(name.as_mut_ptr()), &mut len, None, windows::core::PWSTR::null(), None, None)
                .is_err()
            {
                break;
            }
            i += 1;
            let sub = PCWSTR(name.as_ptr());

            let mut path = [0u16; 520];
            let mut size = (path.len() * 2) as u32;
            if RegGetValueW(root, sub, w!("ExecutablePath"), RRF_RT_REG_SZ, None, Some(path.as_mut_ptr() as *mut _), Some(&mut size))
                .is_err()
            {
                continue;
            }
            let end = path.iter().position(|&c| c == 0).unwrap_or(path.len());
            if String::from_utf16_lossy(&path[..end]).to_lowercase() != exe {
                continue;
            }
            let already = RegGetValueW(root, sub, w!("IsPromoted"), RRF_RT_REG_DWORD, None, None, None).is_ok();
            if !already {
                let one = 1u32;
                let _ = RegSetKeyValueW(root, sub, w!("IsPromoted"), REG_DWORD.0, Some(&one as *const u32 as *const _), 4);
            }
        }
        let _ = RegCloseKey(root);
    }
}
