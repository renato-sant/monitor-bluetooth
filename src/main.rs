#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod battery;
mod config;
mod icon;
mod install;
mod tray;

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, SetProcessWorkingSetSize};
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use battery::Device;
use config::Config;

const WM_SCAN_DONE: u32 = WM_APP + 2;
const TIMER_POLL: usize = 1;
const TIMER_DEVCHANGE: usize = 2;
const TIMER_PROMOTE: usize = 3;
const APP_KEY: &str = "__app__";
const NIN_SELECT: u32 = WM_USER;
const NIN_KEYSELECT: u32 = WM_USER + 1;
const DBT_DEVNODES_CHANGED: usize = 0x0007;
const PBT_APMRESUMEAUTOMATIC: usize = 0x0012;

static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

struct TrayIcon {
    uid: u32,
    name: String,
    level: Option<u8>,
    hicon: HICON,
    low_notified: bool,
}

struct App {
    hwnd: HWND,
    cfg: Config,
    /// Última leitura, incluindo dispositivos ocultos.
    devices: Vec<Device>,
    icons: HashMap<String, TrayIcon>,
    scanning: Arc<AtomicBool>,
    scanned_once: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

/// uID estável por dispositivo (o Windows lembra a posição do ícone por exe+uID).
fn uid_for(key: &str) -> u32 {
    if key == APP_KEY {
        return 1;
    }
    let mut h: u32 = 0x811c_9dc5;
    for b in key.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h | 0x1000
}

fn icon_size() -> i32 {
    unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()) }
}

impl App {
    /// Deixa a bandeja igual à última leitura. `force` redesenha tudo (tema/DPI mudou).
    /// Retorna true se algum ícone novo foi adicionado.
    fn sync(&mut self, force: bool) -> bool {
        let mut added = false;
        let size = icon_size();
        let light = autostart::light_theme();

        let mut wanted: Vec<(String, String, Option<u8>)> = self
            .devices
            .iter()
            .filter(|d| !self.cfg.hidden.contains(&d.key))
            .map(|d| (d.key.clone(), d.name.clone(), Some(d.level)))
            .collect();
        if wanted.is_empty() {
            // Sem dispositivos: mantém um ícone neutro para ter acesso ao menu.
            let tip = if self.scanned_once { "MonitorBT — nenhum dispositivo" } else { "MonitorBT — procurando…" };
            wanted.push((APP_KEY.into(), tip.into(), None));
        }

        let keep: Vec<&str> = wanted.iter().map(|w| w.0.as_str()).collect();
        let gone: Vec<String> = self.icons.keys().filter(|k| !keep.contains(&k.as_str())).cloned().collect();
        for k in gone {
            if let Some(ic) = self.icons.remove(&k) {
                tray::delete(self.hwnd, ic.uid);
                unsafe {
                    let _ = DestroyIcon(ic.hicon);
                }
            }
        }

        // O Windows coloca cada ícone novo à esquerda: adiciona de trás para frente
        // para ficarem em ordem alfabética da esquerda para a direita.
        for (key, name, level) in wanted.into_iter().rev() {
            let tip = match level {
                Some(l) => format!("{name} — {l}%"),
                None => name.clone(),
            };
            match self.icons.get_mut(&key) {
                Some(ic) => {
                    if force || ic.level != level || ic.name != name {
                        let new = icon::make(size, level, light);
                        tray::modify(self.hwnd, ic.uid, new, &tip);
                        unsafe {
                            let _ = DestroyIcon(ic.hicon);
                        }
                        ic.hicon = new;
                        ic.level = level;
                        ic.name = name.clone();
                    }
                }
                None => {
                    let uid = uid_for(&key);
                    let hicon = icon::make(size, level, light);
                    tray::add(self.hwnd, uid, hicon, &tip);
                    added = true;
                    self.icons.insert(key.clone(), TrayIcon { uid, name: name.clone(), level, hicon, low_notified: false });
                }
            }

            // Aviso de bateria baixa (uma vez por descarga; rearma acima de limiar+5).
            if let (Some(l), Some(ic)) = (level, self.icons.get_mut(&key)) {
                let th = self.cfg.threshold;
                if l < th && !ic.low_notified {
                    ic.low_notified = true;
                    tray::balloon(self.hwnd, ic.uid, "Bateria baixa", &format!("{name}: {l}%"));
                } else if l >= th.saturating_add(5) {
                    ic.low_notified = false;
                }
            }
        }
        added
    }

    /// Explorer reiniciou: recria todos os ícones.
    fn readd_all(&mut self) {
        for ic in self.icons.values() {
            let tip = match ic.level {
                Some(l) => format!("{} — {l}%", ic.name),
                None => ic.name.clone(),
            };
            tray::add(self.hwnd, ic.uid, ic.hicon, &tip);
        }
    }

    fn remove_all(&mut self) {
        for (_, ic) in self.icons.drain() {
            tray::delete(self.hwnd, ic.uid);
            unsafe {
                let _ = DestroyIcon(ic.hicon);
            }
        }
    }

    fn start_scan(&self) {
        if self.scanning.swap(true, Ordering::SeqCst) {
            return;
        }
        let flag = self.scanning.clone();
        let hwnd = self.hwnd.0 as isize;
        std::thread::spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            let devices = battery::scan();
            let ptr = Box::into_raw(Box::new(devices));
            flag.store(false, Ordering::SeqCst);
            unsafe {
                let hwnd = HWND(hwnd as *mut _);
                if PostMessageW(hwnd, WM_SCAN_DONE, WPARAM(0), LPARAM(ptr as isize)).is_err() {
                    drop(Box::from_raw(ptr));
                }
            }
        });
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

const CMD_REFRESH: usize = 1;
const CMD_AUTOSTART: usize = 2;
const CMD_EXIT: usize = 3;
const CMD_DEVICE: usize = 1000;

fn show_menu(hwnd: HWND) {
    let Some((devices, hidden)) = with_app(|a| (a.devices.clone(), a.cfg.hidden.clone())) else { return };
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        if devices.is_empty() {
            let t = wide("Nenhum dispositivo com bateria");
            let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, PCWSTR(t.as_ptr()));
        }
        for (i, d) in devices.iter().enumerate() {
            let t = wide(&format!("{}  ({}%)", d.name, d.level));
            let mut flags = MF_STRING;
            if !hidden.contains(&d.key) {
                flags |= MF_CHECKED;
            }
            let _ = AppendMenuW(menu, flags, CMD_DEVICE + i, PCWSTR(t.as_ptr()));
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, CMD_REFRESH, w!("Atualizar agora"));
        let auto_flags = if autostart::is_enabled() { MF_STRING | MF_CHECKED } else { MF_STRING };
        let _ = AppendMenuW(menu, auto_flags, CMD_AUTOSTART, w!("Iniciar com o Windows"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, CMD_EXIT, w!("Sair"));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, 0, hwnd, None);
        let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);

        match cmd.0 as usize {
            0 => {}
            CMD_REFRESH => {
                with_app(|a| a.start_scan());
            }
            CMD_AUTOSTART => autostart::set(!autostart::is_enabled()),
            CMD_EXIT => {
                let _ = DestroyWindow(hwnd);
            }
            c if c >= CMD_DEVICE => {
                if let Some(d) = devices.get(c - CMD_DEVICE) {
                    with_app(|a| {
                        if !a.cfg.hidden.remove(&d.key) {
                            a.cfg.hidden.insert(d.key.clone());
                        }
                        a.cfg.save();
                        if a.sync(false) {
                            SetTimer(hwnd, TIMER_PROMOTE, 3000, None);
                        }
                    });
                }
            }
            _ => {}
        }
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TIMER => {
                if wparam.0 == TIMER_PROMOTE {
                    let _ = KillTimer(hwnd, TIMER_PROMOTE);
                    autostart::promote_tray_icons();
                    return LRESULT(0);
                }
                if wparam.0 == TIMER_DEVCHANGE {
                    let _ = KillTimer(hwnd, TIMER_DEVCHANGE);
                }
                with_app(|a| a.start_scan());
                LRESULT(0)
            }
            WM_SCAN_DONE => {
                let devices = *Box::from_raw(lparam.0 as *mut Vec<Device>);
                with_app(|a| {
                    a.devices = devices;
                    a.scanned_once = true;
                    if a.sync(false) {
                        // O Explorer registra o ícone novo com um pequeno atraso.
                        SetTimer(hwnd, TIMER_PROMOTE, 3000, None);
                    }
                });
                // Devolve ao sistema as páginas usadas só durante a leitura.
                let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
                LRESULT(0)
            }
            tray::WM_TRAY => {
                let ev = (lparam.0 as u32) & 0xFFFF;
                if ev == WM_CONTEXTMENU || ev == NIN_SELECT || ev == NIN_KEYSELECT {
                    show_menu(hwnd);
                }
                LRESULT(0)
            }
            WM_DEVICECHANGE => {
                // Dispositivo conectou/desconectou: relê em alguns segundos
                // (o Windows demora um pouco para preencher a bateria).
                if wparam.0 == DBT_DEVNODES_CHANGED {
                    SetTimer(hwnd, TIMER_DEVCHANGE, 4000, None);
                }
                LRESULT(1)
            }
            WM_POWERBROADCAST => {
                if wparam.0 == PBT_APMRESUMEAUTOMATIC {
                    SetTimer(hwnd, TIMER_DEVCHANGE, 8000, None);
                }
                LRESULT(1)
            }
            WM_SETTINGCHANGE | WM_DISPLAYCHANGE | WM_DPICHANGED => {
                with_app(|a| a.sync(true));
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_DESTROY => {
                with_app(|a| a.remove_all());
                PostQuitMessage(0);
                LRESULT(0)
            }
            m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => {
                with_app(|a| a.readd_all());
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn main() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        let cfg = {
            let mut cfg = Config::load();
            if install::maybe_install(&mut cfg) {
                return; // a cópia instalada assumiu
            }
            cfg
        };

        // Instância única.
        let _mutex = CreateMutexW(None, true, w!("Local\\MonitorBT.single"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }

        let hinstance = GetModuleHandleW(None).unwrap_or_default();
        let class = w!("MonitorBTWnd");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance.into(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassW(&wc);
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);

        // Janela top-level oculta (precisa ser top-level para receber TaskbarCreated
        // e WM_DEVICECHANGE).
        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("MonitorBT"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            hinstance,
            None,
        ) else {
            return;
        };

        let interval_ms = cfg.interval_min.saturating_mul(60_000);
        APP.with(|a| {
            *a.borrow_mut() = Some(App {
                hwnd,
                cfg,
                devices: Vec::new(),
                icons: HashMap::new(),
                scanning: Arc::new(AtomicBool::new(false)),
                scanned_once: false,
            })
        });

        with_app(|a| a.start_scan());
        SetTimer(hwnd, TIMER_POLL, interval_ms, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
