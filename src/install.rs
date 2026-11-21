//! Autoinstalação: na primeira execução fora da pasta de instalação, oferece
//! copiar o .exe para %LOCALAPPDATA%\Programs\MonitorBT (por usuário, sem admin),
//! liga o início com o Windows e abre a cópia instalada.

use std::path::PathBuf;
use std::time::Duration;

use windows::core::{w, HSTRING};
use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::autostart;
use crate::config::Config;

pub fn install_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Programs").join("MonitorBT"))
}

fn same_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy())
}

/// Retorna true se o processo atual deve encerrar (a cópia instalada foi aberta).
pub fn maybe_install(cfg: &mut Config) -> bool {
    // Builds de debug nunca se instalam.
    if cfg!(debug_assertions) {
        return false;
    }
    let (Ok(exe), Some(dir)) = (std::env::current_exe(), install_dir()) else { return false };
    let target = dir.join("monitorbt.exe");
    if same_path(&exe, &target) {
        return false;
    }

    let already = target.exists();
    if !already && !cfg.ask_install {
        return false;
    }

    let text = if already {
        format!("Atualizar o MonitorBT instalado em:\n{}\n\ncom esta versão?", dir.display())
    } else {
        format!(
            "Instalar o MonitorBT em:\n{}\n\nEle vai iniciar junto com o Windows. Depois disso você pode apagar este arquivo.",
            dir.display()
        )
    };
    let answer = unsafe {
        MessageBoxW(
            HWND::default(),
            &HSTRING::from(text),
            w!("MonitorBT"),
            MB_YESNO | MB_ICONQUESTION | MB_SETFOREGROUND,
        )
    };
    if answer != IDYES {
        if !already {
            cfg.ask_install = false;
            cfg.save();
        }
        return false;
    }

    close_running_instance();

    let ok = std::fs::create_dir_all(&dir).is_ok() && copy_with_retry(&exe, &target);
    if !ok {
        unsafe {
            MessageBoxW(
                HWND::default(),
                &HSTRING::from(format!("Não foi possível copiar para:\n{}", target.display())),
                w!("MonitorBT"),
                MB_OK | MB_ICONERROR,
            );
        }
        return false;
    }

    autostart::set_path(&target);
    std::process::Command::new(&target).spawn().is_ok()
}

fn copy_with_retry(from: &std::path::Path, to: &std::path::Path) -> bool {
    for _ in 0..10 {
        if std::fs::copy(from, to).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    false
}

/// Fecha a instância que estiver rodando (para poder substituir o .exe).
fn close_running_instance() {
    unsafe {
        let Ok(hwnd) = FindWindowW(w!("MonitorBTWnd"), None) else { return };
        if hwnd.0.is_null() {
            return;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let process = OpenProcess(PROCESS_SYNCHRONIZE, false, pid).ok();
        let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
        if let Some(h) = process {
            WaitForSingleObject(h, 5000);
            let _ = CloseHandle(h);
        }
    }
}
