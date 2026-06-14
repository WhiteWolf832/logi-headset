// logi-headset — Logitech G733 mute-button remap + LED control (HID++).
// Copyright (C) 2026  WhiteWolf832
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Demon : transforme le bouton mute du casque Logitech G733 en Play/Pause,
//! et eteint ses LED. Port Rust (libc seul) du script Python g733_playpause.py.
//!
//! Principe (tout en HID++, comme G HUB) :
//!   1. auto-detecte le hidraw d'un casque Logitech (046d) exposant la feature
//!      G-keys (0x8010), quel que soit le PID,
//!   2. active la "diversion" de la G-key (feature 0x8010, fonction 2) : le bouton
//!      cesse de couper le micro et envoie une notification HID++,
//!   3. selon la config, regle les LED (feature 0x8070, fonction 3 : couleur fixe),
//!   4. cree un clavier virtuel uinput,
//!   5. a chaque appui (front montant), injecte la touche configuree (def. Play/Pause) ;
//!      si key_double est defini, un double-clic injecte une 2e touche (le simple clic
//!      est alors differe du delai double_ms (defaut 1000 ms), le temps de detecter
//!      un eventuel 2e appui),
//!   6. surveille la batterie (feature proprietaire HID++ 0x1f20 du G733 = tension,
//!      sinon 0x1004/0x1000 = %) et notifie le bureau (notify-send) quand la charge
//!      passe sous un seuil (def. 15 %).
//!
//! Config : ~/.config/logi-headset/config (touche, mode LED, couleur, seuil
//! batterie) — voir --help.
//! Le casque etant sans fil, l'etat (diversion + LED) est perdu a l'extinction ; le
//! demon le reapplique sur la notif d'allumage et periodiquement (toutes les 5 s).
//! NB : le G533 peut mettre jusqu'a ~1-2 min apres l'allumage avant d'accepter la
//! diversion (quirk firmware) ; le re-assert periodique la retablit des qu'elle prend.
//!
//! Acces requis : /dev/hidraw* + /dev/uinput (regle udev uaccess -> sans root).

use std::ffi::CString;
use std::io::Write;
use std::os::unix::io::RawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

// ---- G733 / HID++ -----------------------------------------------------------
const VID: u16 = 0x046D;
const PID: u16 = 0x0B1F;
const DEV_INDEX: u8 = 0xFF;
const LONG_REPORT_ID: u8 = 0x11;
const LONG_LEN: usize = 20;
const GKEY_FEATURE: u16 = 0x8010;
const FN_DIVERT: u8 = 0x02;
const RGB_FEATURE: u16 = 0x8070;
const FN_SET_FIXED: u8 = 0x03;
const MODE_FIXED: u8 = 0x01;
const RGB_INDEX_FALLBACK: u8 = 0x04;
const LED_ZONE_DELAY_MS: u64 = 60; // pause entre les 2 zones (anti-race ; 50 ms ok au test)
// La feature d'etat (notifs reveil/extinction) est la feature proprietaire 0x1f20
// (= BATT_VOLTAGE) ; son index est resolu dynamiquement a la connexion (8 sur le
// G733, potentiellement autre sur un autre casque Logitech) — voir serve().
const WAKE_OFF: u8 = 0x00; // byte4 de la notif : 0x00 = extinction, !=0 = allumage
const DEFAULT_DOUBLE_MS: u64 = 1000; // fenetre de double-clic par defaut (override: config double_ms)
const REASSERT_SECONDS: f64 = 5.0; // filet de securite ; le re-assert instantane = sur la notif

/// Ou les utilisateurs envoient leur diagnostic (affiche par `--diagnose`).
/// PLACEHOLDER : a ajuster quand le depot public sera cree.
const ISSUES_URL: &str = "https://github.com/WhiteWolf832/logi-headset/issues";

// ---- batterie (HID++) -------------------------------------------------------
// Le G733 n'expose aucune feature batterie standard ; il porte une feature
// proprietaire Logitech 0x1f20 dont la fonction 0 renvoie une TENSION (mV, gros-
// boutiste en p0:p1). On garde 0x1004/0x1000 en repli pour d'autres peripheriques.
const BATT_VOLTAGE: u16 = 0x1F20; // G733 : tension (mV) via fonction 0
const BATT_UNIFIED: u16 = 0x1004; // repli standard : % via fonction 1
const BATT_LEVEL: u16 = 0x1000; // repli standard ancien : % via fonction 0
const BATT_WARN_DEFAULT: u8 = 15; // seuil d'alerte en % (0 = surveillance desactivee)
const BATT_REARM_MARGIN: u8 = 10; // hysteresis : re-arme l'alerte au-dessus de (seuil + marge)
const BATT_FIRST_POLL_SECS: u64 = 5; // 1re mesure peu apres la connexion (charge affichee vite dans la GUI)
const BATT_POLL_SECS: u64 = 300; // puis une mesure toutes les 5 min

// Courbes tension (mV) -> charge (%). Logitech ne publie pas ses courbes ; celles par
// modele (G533, G633/G733) viennent du projet HeadsetControl (GPL,
// lib/devices/protocols/logitech_calibrations.hpp) — bien plus justes qu'une courbe
// Li-ion generique (qui surestime fortement : un G533 a 3,69 V = ~7 %, pas ~42 %).
// La courbe generique sert de repli pour un modele inconnu. Points strictement
// decroissants en tension ; interpolation lineaire (la tension brute est aussi affichee).
type Calib = &'static [(u16, u8)];

const CURVE_G533: [(u16, u8); 6] = [(4200, 100), (3850, 50), (3790, 30), (3750, 20), (3680, 5), (3330, 0)];
// G733 partage la calibration du G633 (comme dans HeadsetControl).
const CURVE_G633: [(u16, u8); 8] =
    [(4100, 100), (3950, 80), (3850, 60), (3750, 40), (3650, 20), (3500, 10), (3300, 5), (3150, 0)];
const CURVE_GENERIC: [(u16, u8); 12] = [
    (4200, 100), (4100, 92), (4000, 81), (3900, 69), (3800, 56), (3700, 44), (3650, 36), (3600, 28),
    (3550, 20), (3500, 13), (3400, 6), (3200, 0),
];

/// Choisit la courbe de calibration selon le nom de modele (repli : generique).
fn curve_for_model(name: Option<&str>) -> Calib {
    let n = name.unwrap_or("").to_ascii_uppercase();
    if n.contains("G533") {
        &CURVE_G533
    } else if n.contains("G733") || n.contains("G633") || n.contains("G933") || n.contains("G935") {
        &CURVE_G633
    } else {
        &CURVE_GENERIC
    }
}

/// Convertit une tension (mV) en charge approximative (%) via la courbe donnee.
fn voltage_to_percent(curve: Calib, mv: u16) -> u8 {
    if mv >= curve[0].0 {
        return 100;
    }
    for w in curve.windows(2) {
        let (hi_mv, hi_pct) = w[0];
        let (lo_mv, lo_pct) = w[1];
        if mv >= lo_mv {
            // interpolation lineaire entre (lo_mv,lo_pct) et (hi_mv,hi_pct)
            let span = (hi_mv - lo_mv) as u32;
            let frac = (mv - lo_mv) as u32;
            let delta = (hi_pct - lo_pct) as u32;
            return (lo_pct as u32 + frac * delta / span) as u8;
        }
    }
    0
}

// ---- uinput / input-event ---------------------------------------------------
const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const KEY_PLAYPAUSE: u16 = 164;
const UI_SET_EVBIT: libc::c_ulong = 0x40045564;
const UI_SET_KEYBIT: libc::c_ulong = 0x40045565;
const UI_DEV_CREATE: libc::c_ulong = 0x5501;
const UI_DEV_DESTROY: libc::c_ulong = 0x5502;
const UINPUT_PATHS: [&str; 2] = ["/dev/uinput", "/dev/input/uinput"];

static STOP: AtomicBool = AtomicBool::new(false);

macro_rules! log {
    ($($arg:tt)*) => {{
        println!("[logi-headset] {}", format!($($arg)*));
        let _ = std::io::stdout().flush();
    }};
}

extern "C" fn on_signal(_sig: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

fn stopped() -> bool {
    STOP.load(Ordering::SeqCst)
}

// ---- config -----------------------------------------------------------------
#[derive(Clone, Copy, Debug)]
enum LedMode {
    Keep,
    Off,
    Color,
}

#[derive(Clone, Copy)]
struct Config {
    key_code: u16,
    double_code: Option<u16>, // touche du double-clic ; None = double-clic desactive
    double_ms: u64,           // fenetre de detection du double-clic
    led_mode: LedMode,
    led_color: (u8, u8, u8),
    battery_warn: u8,         // seuil d'alerte batterie en % (0 = desactive)
}

impl Default for Config {
    fn default() -> Self {
        Config {
            key_code: KEY_PLAYPAUSE,
            double_code: None,
            double_ms: DEFAULT_DOUBLE_MS,
            led_mode: LedMode::Keep,
            led_color: (0, 0, 0),
            battery_warn: BATT_WARN_DEFAULT,
        }
    }
}

/// Nom de touche -> keycode evdev. Accepte aussi un code numerique brut.
fn key_name_to_code(name: &str) -> Option<u16> {
    Some(match name.trim().to_lowercase().as_str() {
        "playpause" | "play_pause" => 164,           // KEY_PLAYPAUSE
        "next" | "nextsong" => 163,                  // KEY_NEXTSONG
        "previous" | "prev" | "previoussong" => 165, // KEY_PREVIOUSSONG
        "stop" | "stopcd" => 166,                    // KEY_STOPCD
        "mute" => 113,                               // KEY_MUTE
        "micmute" | "mic_mute" => 248,               // KEY_MICMUTE
        "volumeup" | "volume_up" => 115,             // KEY_VOLUMEUP
        "volumedown" | "volume_down" => 114,         // KEY_VOLUMEDOWN
        other => return other.parse::<u16>().ok(),
    })
}

fn parse_color(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    Some((
        u8::from_str_radix(&s[0..2], 16).ok()?,
        u8::from_str_radix(&s[2..4], 16).ok()?,
        u8::from_str_radix(&s[4..6], 16).ok()?,
    ))
}

/// $XDG_CONFIG_HOME (ou ~/.config) /logi-headset/config.
fn default_config_path() -> std::path::PathBuf {
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
            std::path::Path::new(&home).join(".config")
        });
    dir.join("logi-headset").join("config")
}

fn load_config(path: &std::path::Path) -> Config {
    let mut cfg = Config::default();
    let txt = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return cfg, // pas de fichier -> defauts
    };
    for line in txt.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = match line.split_once('=') {
            Some(kv) => (kv.0.trim(), kv.1.trim()),
            None => continue,
        };
        match k {
            "key" => {
                if let Some(c) = key_name_to_code(v) {
                    cfg.key_code = c;
                }
            }
            "key_double" => {
                cfg.double_code = match v.to_lowercase().as_str() {
                    "" | "none" | "aucune" => None,
                    s => key_name_to_code(s),
                };
            }
            "double_ms" => {
                if let Ok(ms) = v.parse::<u64>() {
                    cfg.double_ms = ms.clamp(100, 3000);
                }
            }
            "leds" => {
                cfg.led_mode = match v.to_lowercase().as_str() {
                    "off" => LedMode::Off,
                    "color" => LedMode::Color,
                    _ => LedMode::Keep,
                }
            }
            "led_color" => {
                if let Some(c) = parse_color(v) {
                    cfg.led_color = c;
                }
            }
            "battery_warn" | "batterie" => {
                cfg.battery_warn = match v.to_lowercase().as_str() {
                    "off" | "none" | "aucune" | "non" => 0,
                    s => s.parse::<u8>().unwrap_or(cfg.battery_warn).min(100),
                };
            }
            _ => {}
        }
    }
    cfg
}

/// Couleur a appliquer selon le mode, ou None si on ne touche pas aux LED.
fn led_target(cfg: &Config) -> Option<(u8, u8, u8)> {
    match cfg.led_mode {
        LedMode::Keep => None,
        LedMode::Off => Some((0, 0, 0)),
        LedMode::Color => Some(cfg.led_color),
    }
}

/// Fichier d'etat lu par la GUI : $XDG_RUNTIME_DIR/logi-headset.status.
fn status_path() -> std::path::PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("logi-headset.status")
}

/// Fichier de charge batterie publie pour la GUI (libelle pret a afficher).
fn battery_status_path() -> std::path::PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("logi-headset.battery")
}

/// Fichier du nom de modele du casque, publie pour la GUI.
fn device_status_path() -> std::path::PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("logi-headset.device")
}

/// Fichier de capacites du casque publie pour la GUI (ex. `rgb=1`).
fn caps_status_path() -> std::path::PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("logi-headset.caps")
}

fn set_status(s: &str) {
    let _ = std::fs::write(status_path(), s);
    // Hors connexion, ces infos ne sont plus valides -> on les efface.
    if s != "connecte" {
        let _ = std::fs::remove_file(battery_status_path());
        let _ = std::fs::remove_file(device_status_path());
        let _ = std::fs::remove_file(caps_status_path());
    }
}

// ---- couche HID++ -----------------------------------------------------------
/// Auto-detecte un casque Logitech exposant la feature G-keys (0x8010), quel que
/// soit son PID. Scanne les hidraw du vendor 046d et garde celui qui repond a un
/// root.getFeature(0x8010) — auto-validant : les recepteurs (interroges a devIndex
/// 0xFF) et les interfaces audio ne repondent pas et sont ignores.
/// Retourne (fd RDWR ouvert, chemin, index GKEY).
fn find_headset() -> Option<(RawFd, String, u8)> {
    let vendor = format!("HID_ID=0003:{:08X}:", VID as u32);
    let base = "/sys/class/hidraw";
    let mut names: Vec<String> = std::fs::read_dir(base)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    for n in names {
        let uevent = format!("{}/{}/device/uevent", base, n);
        match std::fs::read_to_string(&uevent) {
            Ok(txt) if txt.to_uppercase().contains(&vendor) => {}
            _ => continue, // pas un peripherique Logitech
        }
        let path = format!("/dev/{}", n);
        let cpath = match CString::new(path.clone()) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let fd = unsafe { libc::open(cpath.as_ptr(), libc::O_RDWR) };
        if fd < 0 {
            log!(
                "ouverture {} impossible: {} (droits ? regle udev uaccess ?)",
                path,
                std::io::Error::last_os_error()
            );
            continue;
        }
        match query_feature_index(fd, GKEY_FEATURE) {
            Some(gkey_index) => return Some((fd, path, gkey_index)),
            None => unsafe {
                libc::close(fd); // ce noeud Logitech n'a pas de G-keys -> suivant
            },
        }
    }
    None
}

fn hidpp_long(feature_index: u8, fb: u8, params: &[u8]) -> [u8; LONG_LEN] {
    let mut buf = [0u8; LONG_LEN];
    buf[0] = LONG_REPORT_ID;
    buf[1] = DEV_INDEX;
    buf[2] = feature_index;
    buf[3] = fb;
    for (i, &p) in params.iter().take(16).enumerate() {
        buf[4 + i] = p;
    }
    buf
}

fn write_report(fd: RawFd, report: &[u8]) {
    unsafe {
        libc::write(fd, report.as_ptr() as *const libc::c_void, report.len());
    }
}

/// Attend une lecture sur `fd` jusqu'a `timeout_ms`. true si des donnees sont pretes.
fn poll_in(fd: RawFd, timeout_ms: i32) -> bool {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let n = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
    n > 0 && (pfd.revents & libc::POLLIN) != 0
}

/// root.getFeature(feature_id) -> index de la feature. None si echec/absente.
fn query_feature_index(fd: RawFd, feature_id: u16) -> Option<u8> {
    let fb: u8 = 0x0F; // fonction 0 du root, swid 0x0F
    let params = [(feature_id >> 8) as u8, (feature_id & 0xFF) as u8];
    write_report(fd, &hidpp_long(0x00, fb, &params));
    let deadline = Instant::now() + Duration::from_millis(600);
    loop {
        let now = Instant::now();
        if now >= deadline {
            return None;
        }
        let remaining = (deadline - now).as_millis() as i32;
        if !poll_in(fd, remaining) {
            return None;
        }
        let mut buf = [0u8; 64];
        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n < 5 {
            continue;
        }
        if buf[0] == LONG_REPORT_ID && buf[1] == DEV_INDEX && buf[2] == 0x00 && buf[3] == fb {
            let idx = buf[4];
            return if idx != 0 { Some(idx) } else { None };
        }
    }
}

/// Envoie une requete HID++ et attend la reponse correlee (meme index + meme
/// fonction/swid). Renvoie les 16 octets de parametres, ou None (timeout/erreur).
/// Utilitaire de `--diagnose` (le chemin nominal a ses propres lectures).
fn hidpp_request(fd: RawFd, index: u8, function: u8, params: &[u8]) -> Option<[u8; 16]> {
    let fb = (function << 4) | 0x0F;
    write_report(fd, &hidpp_long(index, fb, params));
    let deadline = Instant::now() + Duration::from_millis(600);
    loop {
        let now = Instant::now();
        if now >= deadline {
            return None;
        }
        let remaining = (deadline - now).as_millis() as i32;
        if !poll_in(fd, remaining) {
            return None;
        }
        let mut buf = [0u8; 64];
        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n < 20 {
            continue;
        }
        // erreur HID++ : buf[2]=0xff, buf[3]=index demande, buf[4]=fb
        if buf[0] == LONG_REPORT_ID && buf[2] == 0xFF && buf[3] == index && buf[4] == fb {
            return None;
        }
        if buf[0] == LONG_REPORT_ID && buf[2] == index && buf[3] == fb {
            let mut out = [0u8; 16];
            out.copy_from_slice(&buf[4..20]);
            return Some(out);
        }
    }
}

/// Active/desactive la diversion (fire-and-forget, sans attendre l'ack).
fn set_divert(fd: RawFd, gkey_index: u8, enable: bool) {
    let fb = (FN_DIVERT << 4) | 0x0F;
    write_report(fd, &hidpp_long(gkey_index, fb, &[if enable { 0x01 } else { 0x00 }]));
}

/// Lit le nom de modele via la feature DeviceName (0x0005) : fonction 0 = longueur,
/// fonction 1 = morceaux de 16 caracteres a partir d'un offset. None si absente.
fn read_device_name(fd: RawFd) -> Option<String> {
    let index = query_feature_index(fd, 0x0005)?;
    let count = hidpp_request(fd, index, 0x00, &[])?[0] as usize;
    if count == 0 {
        return None;
    }
    let mut name = String::new();
    while name.len() < count && name.len() < 64 {
        let r = hidpp_request(fd, index, 0x01, &[name.len() as u8])?;
        let before = name.len();
        for &b in &r {
            if name.len() >= count || b == 0 {
                break;
            }
            name.push(b as char);
        }
        if name.len() == before {
            break; // pas de progres -> on s'arrete
        }
    }
    let name = name.trim().to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Regle les deux zones LED a une couleur fixe. Fire-and-forget.
/// Une pause separe les 2 ecritures : sinon le casque avale parfois la 2e
/// (race confirmee par test -> une seule oreillette prenait la couleur).
fn set_led_color(fd: RawFd, rgb_index: u8, rgb: (u8, u8, u8)) {
    let fb = (FN_SET_FIXED << 4) | 0x0F;
    for zone in [0x00u8, 0x01u8] {
        write_report(
            fd,
            &hidpp_long(rgb_index, fb, &[zone, MODE_FIXED, rgb.0, rgb.1, rgb.2, 0x02]),
        );
        std::thread::sleep(Duration::from_millis(LED_ZONE_DELAY_MS));
    }
}

// ---- surveillance batterie --------------------------------------------------
/// Surveille la charge du casque via HID++ et emet UNE notification desktop quand
/// elle passe sous le seuil. Anti-spam par hysteresis : re-armee quand la charge
/// remonte au-dessus de (seuil + marge) — ce qui couvre aussi la mise en charge,
/// sans dependre d'un octet de statut dont la semantique varie selon la feature.
///
/// La requete est envoyee en fire-and-forget ; sa reponse est captee par la boucle
/// de lecture principale (comme les notifs G-key/etat) — donc aucune lecture
/// synchrone ne risque d'avaler un appui bouton.
struct Battery {
    index: u8,          // index de la feature batterie sur ce casque
    fb: u8,             // (fonction << 4) | swid : sert a emettre ET a correler la reponse
    voltage_mode: bool, // true = 0x1f20 (tension mV) ; false = feature standard (% direct)
    warn: u8,           // seuil d'alerte en %
    warned: bool,       // alerte deja emise depuis le dernier re-armement
    next_poll: Instant,
    curve: Calib,       // courbe tension->% selon le modele (mode tension)
}

impl Battery {
    /// Detecte la feature batterie : 0x1f20 (G733, tension) en priorite, sinon les
    /// standards 0x1004 / 0x1000 (pourcentage). `None` si le casque n'expose aucune
    /// de ces features. La surveillance tourne toujours (pour publier la charge a la
    /// GUI) ; `warn` ne controle que la notification (0 = surveiller sans alerter).
    fn detect(fd: RawFd, warn: u8, curve: Calib) -> Option<Battery> {
        // (featureId, fonction de lecture, mode tension ?)
        let candidates = [
            (BATT_VOLTAGE, 0x00u8, true),
            (BATT_UNIFIED, 0x01u8, false),
            (BATT_LEVEL, 0x00u8, false),
        ];
        for (feature, func, voltage_mode) in candidates {
            if let Some(index) = query_feature_index(fd, feature) {
                let unit = if voltage_mode { "tension" } else { "pourcentage" };
                let alert = if warn > 0 {
                    format!("alerte < {warn} %")
                } else {
                    "sans alerte".to_string()
                };
                log!("batterie: feature 0x{feature:04x} @ index {index} ({unit}) — {alert}");
                return Some(Battery {
                    index,
                    fb: (func << 4) | 0x0F,
                    voltage_mode,
                    warn,
                    warned: false,
                    next_poll: Instant::now() + Duration::from_secs(BATT_FIRST_POLL_SECS),
                    curve,
                });
            }
        }
        log!("batterie: aucune feature 0x1f20/0x1004/0x1000");
        None
    }

    fn due(&self) -> bool {
        Instant::now() >= self.next_poll
    }

    /// Demande l'etat (fire-and-forget) et programme la prochaine mesure.
    fn request(&mut self, fd: RawFd) {
        write_report(fd, &hidpp_long(self.index, self.fb, &[]));
        self.next_poll = Instant::now() + Duration::from_secs(BATT_POLL_SECS);
    }

    /// Vrai si `buf` est la reponse a notre requete (meme index + meme fonction/swid).
    fn is_response(&self, buf: &[u8]) -> bool {
        buf.len() >= 7 && buf[2] == self.index && buf[3] == self.fb
    }

    /// Parse la reponse, derive (% , libelle affichable) et notifie si besoin.
    /// Mode tension (0x1f20) : mV gros-boutiste en buf[4:6] -> % via la courbe, et le
    /// libelle montre AUSSI la tension brute (le % n'est qu'une estimation).
    /// Mode standard : % direct en buf[4].
    fn handle(&mut self, buf: &[u8]) {
        let (percent, label, charging) = if self.voltage_mode {
            let mv = ((buf[4] as u16) << 8) | buf[5] as u16;
            if mv == 0 {
                return; // lecture invalide -> on ignore
            }
            // Octet de statut : 0x01 = sur batterie ; sinon (ex. 0x03) = en charge.
            // En charge, la tension est gonflee -> le % n'est qu'indicatif.
            let charging = buf[6] != 0x01;
            let pct = voltage_to_percent(self.curve, mv);
            let v = format!("{}.{:02} V", mv / 1000, (mv % 1000) / 10);
            let label = if charging {
                format!("~{pct} % ({v}, en charge)")
            } else {
                format!("~{pct} % ({v})")
            };
            (pct, label, charging)
        } else {
            let pct = buf[4];
            if pct == 0 {
                return; // lecture transitoire / niveau discret non gere -> on ignore
            }
            (pct, format!("{pct} %"), false)
        };
        log!("batterie: {label}");
        let _ = std::fs::write(battery_status_path(), &label); // publie pour la GUI
        // En charge : pas d'alerte (le % est gonfle par la tension de charge), et on
        // re-arme pour pouvoir re-alerter une fois debranche et redescendu sous le seuil.
        if charging {
            self.warned = false;
            return;
        }
        // L'alerte n'est armee que si warn > 0 (sinon percent <= 0 est toujours faux).
        if !self.warned && percent <= self.warn {
            notify_low_battery(&label);
            self.warned = true;
        } else if self.warned && percent >= self.warn.saturating_add(BATT_REARM_MARGIN) {
            self.warned = false; // remonte -> pret a re-alerter au prochain creux
        }
    }
}

/// Notification desktop "batterie faible" via `notify-send` (libnotify) — sans
/// dependance ajoutee. Fire-and-forget : si `notify-send` est absent, on ignore.
/// (SIGCHLD est ignore dans main() -> le processus enfant est auto-recolte.)
fn notify_low_battery(label: &str) {
    let _ = std::process::Command::new("notify-send")
        .args([
            "--urgency=critical",
            "--icon=battery-caution",
            "--app-name=Logitech Headset",
            "Casque Logitech — batterie faible",
        ])
        .arg(format!("Niveau : {label}. Pense a recharger."))
        .spawn();
}

// ---- couche uinput ----------------------------------------------------------
fn open_uinput(keys: &[u16]) -> std::io::Result<RawFd> {
    let mut fd = -1;
    for path in UINPUT_PATHS {
        let c = CString::new(path).unwrap();
        let f = unsafe { libc::open(c.as_ptr(), libc::O_WRONLY | libc::O_NONBLOCK) };
        if f >= 0 {
            fd = f;
            break;
        }
    }
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    unsafe {
        if libc::ioctl(fd, UI_SET_EVBIT, EV_KEY as libc::c_int) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        for &k in keys {
            if libc::ioctl(fd, UI_SET_KEYBIT, k as libc::c_int) < 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
    }
    // struct uinput_user_dev (1116 o) : name[80], input_id{bustype,vendor,product,version},
    // ff_effects_max:u32, puis absmax/min/fuzz/flat[64] (256 i32 a zero).
    let mut dev = [0u8; 1116];
    let name = b"logi-headset remap";
    dev[..name.len()].copy_from_slice(name);
    dev[80..82].copy_from_slice(&0x03u16.to_le_bytes()); // bustype = BUS_USB
    dev[82..84].copy_from_slice(&VID.to_le_bytes());
    dev[84..86].copy_from_slice(&PID.to_le_bytes());
    dev[86..88].copy_from_slice(&1u16.to_le_bytes()); // version
    // offset 88 = ff_effects_max (u32) = 0, reste a zero
    unsafe {
        if libc::write(fd, dev.as_ptr() as *const libc::c_void, dev.len()) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::ioctl(fd, UI_DEV_CREATE) < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    std::thread::sleep(Duration::from_millis(200)); // laisse le systeme enregistrer le device
    Ok(fd)
}

fn push_event(buf: &mut Vec<u8>, etype: u16, code: u16, value: i32) {
    buf.extend_from_slice(&0i64.to_le_bytes()); // tv_sec
    buf.extend_from_slice(&0i64.to_le_bytes()); // tv_usec
    buf.extend_from_slice(&etype.to_le_bytes());
    buf.extend_from_slice(&code.to_le_bytes());
    buf.extend_from_slice(&value.to_le_bytes());
}

fn emit_key(ui_fd: RawFd, code: u16) {
    let mut buf = Vec::with_capacity(24 * 4);
    push_event(&mut buf, EV_KEY, code, 1);
    push_event(&mut buf, EV_SYN, 0, 0);
    push_event(&mut buf, EV_KEY, code, 0);
    push_event(&mut buf, EV_SYN, 0, 0);
    write_report(ui_fd, &buf);
}

fn close_uinput(ui_fd: RawFd) {
    unsafe {
        libc::ioctl(ui_fd, UI_DEV_DESTROY);
        libc::close(ui_fd);
    }
}

// ---- boucle principale ------------------------------------------------------
/// Une session de connexion au casque. Retourne quand le casque disparait ou
/// qu'on demande l'arret.
fn serve(ui_fd: RawFd, cfg: &Config) {
    let (fd, path, gkey_index) = match find_headset() {
        Some(t) => t,
        None => {
            set_status("recherche");
            return;
        }
    };
    log!("casque connecte: {path} (GKEY @ index {gkey_index})");
    set_status("connecte");
    // Nom de modele (DeviceName) — publie pour la GUI, logge, et choisit la courbe batterie.
    let model = read_device_name(fd);
    if let Some(name) = &model {
        log!("modele : {name}");
        let _ = std::fs::write(device_status_path(), name);
    }
    let curve = curve_for_model(model.as_deref());

    set_divert(fd, gkey_index, true);
    log!("diversion activee — bouton mute = touche configuree");
    // Index de la feature d'etat 0x1f20 (notifs de reveil) — dynamique, pour marcher
    // sur n'importe quel casque Logitech HID++, pas seulement le G733 (index 8).
    let state_index = query_feature_index(fd, BATT_VOLTAGE);
    if let Some(si) = state_index {
        log!("feature d'etat (reveil) @ index {si}");
    }
    // Presence de LED RGB (feature 0x8070) — publiee pour la GUI (qui grise la config
    // LED si le casque n'en a pas, ex. G533).
    let rgb_present = query_feature_index(fd, RGB_FEATURE);
    let _ = std::fs::write(
        caps_status_path(),
        format!("rgb={}\n", if rgb_present.is_some() { 1 } else { 0 }),
    );
    let led = led_target(cfg);
    let rgb_index = rgb_present.unwrap_or(RGB_INDEX_FALLBACK);
    if let Some(rgb) = led {
        set_led_color(fd, rgb_index, rgb);
        log!("LED reglees (RGB @ index {rgb_index})");
    }
    let mut battery = Battery::detect(fd, cfg.battery_warn, curve);

    let mut last_assert = Instant::now();
    let mut prev = false;
    let mut restore = true;
    let mut pending: Option<Instant> = None; // 1er clic en attente (mode double-clic)
    while !stopped() {
        // En attente d'un eventuel 2e clic : on se reveille a l'expiration de la fenetre.
        let timeout_ms = match pending {
            Some(t) => (cfg.double_ms as i64 - t.elapsed().as_millis() as i64).clamp(1, 1000) as i32,
            None => 1000,
        };
        let readable = poll_in(fd, timeout_ms);
        if last_assert.elapsed().as_secs_f64() >= REASSERT_SECONDS {
            set_divert(fd, gkey_index, true);
            if let Some(rgb) = led {
                set_led_color(fd, rgb_index, rgb);
            }
            last_assert = Instant::now();
        }
        // Sondage batterie (fire-and-forget) ; la reponse revient plus bas dans la boucle.
        if let Some(b) = &mut battery {
            if b.due() {
                b.request(fd);
            }
        }
        // Fenetre de double-clic expiree sans 2e appui -> c'etait un simple clic.
        if let Some(t) = pending {
            if t.elapsed().as_millis() as u64 >= cfg.double_ms {
                emit_key(ui_fd, cfg.key_code);
                log!("simple clic ({})", cfg.key_code);
                pending = None;
            }
        }
        if !readable {
            continue;
        }
        let mut buf = [0u8; 64];
        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n <= 0 {
            log!("casque deconnecte");
            set_status("recherche");
            restore = false; // l'etat est reset, plus la peine de restaurer
            break;
        }
        let n = n as usize;
        // Reponse a notre requete batterie (buf[3] != 0x00, donc distincte des notifs).
        if let Some(b) = &mut battery {
            if b.is_response(&buf[..n]) {
                b.handle(&buf[..n]);
                continue;
            }
        }
        if n >= 5 && buf[0] == LONG_REPORT_ID && buf[3] == 0x00 {
            if state_index == Some(buf[2]) && buf[4] != WAKE_OFF {
                // Reveil : on re-applique diversion + LED en place. Le G533 peut mettre
                // jusqu'a ~1-2 min apres l'allumage avant d'accepter la diversion ; le
                // re-assert periodique (ci-dessus) la retablit des qu'elle "prend".
                set_divert(fd, gkey_index, true);
                if let Some(rgb) = led {
                    set_led_color(fd, rgb_index, rgb);
                }
                last_assert = Instant::now();
                log!("casque rallume -> etat reapplique");
            } else if buf[2] == gkey_index {
                let pressed = (buf[4] & 0x01) != 0;
                if pressed && !prev {
                    // front montant
                    match cfg.double_code {
                        None => {
                            // pas de double-clic configure -> simple clic immediat
                            emit_key(ui_fd, cfg.key_code);
                            log!("clic ({})", cfg.key_code);
                        }
                        Some(dc) => match pending.take() {
                            // 2e appui dans la fenetre -> double-clic
                            Some(t) if (t.elapsed().as_millis() as u64) <= cfg.double_ms => {
                                emit_key(ui_fd, dc);
                                log!("double-clic ({dc})");
                            }
                            // 1er appui -> on differe (le simple clic part a l'expiration)
                            _ => pending = Some(Instant::now()),
                        },
                    }
                }
                prev = pressed;
            }
        }
    }

    if restore {
        set_divert(fd, gkey_index, false);
        log!("diversion desactivee — bouton mute restaure");
    }
    unsafe {
        libc::close(fd);
    }
}

/// Nom lisible d'une featureId HID++ (sous-ensemble utile au diagnostic).
fn feature_name(fid: u16) -> &'static str {
    match fid {
        0x0000 => "Root",
        0x0001 => "IFeatureSet",
        0x0003 => "DeviceFwVersion",
        0x0005 => "DeviceName",
        0x1000 => "BatteryLevelStatus",
        0x1001 => "BatteryVoltage",
        0x1004 => "UnifiedBattery",
        0x1814 => "ChangeHost",
        0x1b04 => "ReprogControlsV4",
        0x1f20 => "proprietaire (etat + batterie)",
        0x8010 => "G-keys",
        0x8060 => "ReportRate",
        0x8070 => "RGB Effects",
        0x8300 => "Sidetone",
        0x8310 => "EqualizerCfg",
        _ => "",
    }
}

/// Mode `--diagnose` : enumere les features HID++ du casque et affiche un rapport
/// a coller dans une issue GitHub (onboarding d'un nouveau modele). Le service doit
/// etre arrete (la GUI s'en charge) sinon les deux process se disputent le hidraw.
fn diagnose() {
    let (fd, path, gkey_index) = match find_headset() {
        Some(t) => t,
        None => {
            println!("Aucun casque Logitech (feature G-keys 0x8010) detecte.");
            println!("Verifie qu'il est allume et que le service est arrete :");
            println!("  systemctl --user stop logi-headset");
            return;
        }
    };
    println!(
        "=== Diagnostic casque Logitech — logi-headset v{} ===",
        env!("CARGO_PKG_VERSION")
    );
    println!("Peripherique : {path}");
    let model = read_device_name(fd);
    if let Some(name) = &model {
        println!("Modele : {name}");
    }
    let curve = curve_for_model(model.as_deref());
    if let Some(r) = hidpp_request(fd, 0x00, 0x01, &[0x00, 0x00, 0x5A]) {
        println!("Protocole HID++ : v{}.{}", r[0], r[1]);
    }

    println!();
    println!("Index resolus :");
    println!("  G-keys (0x8010)   @ index {gkey_index}");
    // Nombre de G-keys divertibles (fonction 0 = getCount) : 0 => le bouton mute
    // n'est PAS une G-key reprogrammable sur ce modele (mic-mute firmware).
    if let Some(r) = hidpp_request(fd, gkey_index, 0x00, &[]) {
        println!("  G-keys count      : {}  (touches divertibles)", r[0]);
    }
    match query_feature_index(fd, RGB_FEATURE) {
        Some(i) => println!("  RGB LED (0x8070)  @ index {i}"),
        None => println!("  RGB LED (0x8070)  absente"),
    }
    match query_feature_index(fd, BATT_VOLTAGE) {
        Some(i) => println!("  Etat/reveil       @ index {i}  (0x1f20)"),
        None => println!("  Etat/reveil       absent  (0x1f20)"),
    }

    // Batterie : meme detection que le demon (0x1f20 -> 0x1004 -> 0x1000), lue une fois.
    let batt = [
        (BATT_VOLTAGE, 0x00u8, true),
        (BATT_UNIFIED, 0x01u8, false),
        (BATT_LEVEL, 0x00u8, false),
    ]
    .into_iter()
    .find_map(|(f, func, volt)| query_feature_index(fd, f).map(|i| (f, i, func, volt)));
    match batt {
        Some((f, i, func, volt)) => {
            print!("  Batterie (0x{f:04x}) @ index {i}  -> ");
            match hidpp_request(fd, i, func, &[]) {
                Some(r) if volt => {
                    let mv = ((r[0] as u16) << 8) | r[1] as u16;
                    println!(
                        "~{} % ({}.{:02} V)",
                        voltage_to_percent(curve, mv),
                        mv / 1000,
                        (mv % 1000) / 10
                    );
                }
                Some(r) => println!("{} %", r[0]),
                None => println!("lecture indisponible"),
            }
        }
        None => println!("  Batterie          aucune feature (0x1f20/1004/1000)"),
    }

    // Enumeration complete via IFeatureSet (utile pour ajouter un nouveau modele).
    if let Some(ifs) = query_feature_index(fd, 0x0001) {
        if let Some(count) = hidpp_request(fd, ifs, 0x00, &[]).map(|r| r[0]) {
            println!();
            println!("Features ({count}) :");
            for idx in 1..=count {
                if let Some((fid, _ftype)) = hidpp_request(fd, ifs, 0x01, &[idx])
                    .map(|r| (((r[0] as u16) << 8) | r[1] as u16, r[2]))
                {
                    println!("  {idx:>2}  0x{fid:04x}  {}", feature_name(fid));
                }
            }
        }
    }

    println!();
    println!("-> Colle ce rapport dans une issue GitHub :");
    println!("   {ISSUES_URL}");

    unsafe {
        libc::close(fd);
    }
}

/// Mode `--watch` : affiche les rapports HID++ bruts en direct. Appuie sur les
/// boutons du casque pour voir ce qu'ils emettent — outil de debug pour les modeles
/// dont un bouton n'est pas vu. Service arrete requis ; Ctrl-C / SIGTERM pour finir.
fn watch() {
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
    }
    let (fd, path, gkey_index) = match find_headset() {
        Some(t) => t,
        None => {
            println!("Aucun casque detecte (service arrete ?).");
            return;
        }
    };
    set_divert(fd, gkey_index, true); // pour voir d'eventuelles notifs de G-key
    println!("Surveillance HID++ sur {path} — appuie sur les boutons du casque.");
    println!("(aucune ligne = le bouton n'emet rien sur cette interface). Ctrl-C pour finir.\n");
    while !stopped() {
        if poll_in(fd, 500) {
            let mut buf = [0u8; 64];
            let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n > 0 {
                let hex: Vec<String> = buf[..n as usize].iter().map(|b| format!("{b:02x}")).collect();
                println!("  {}", hex.join(" "));
            }
        }
    }
    set_divert(fd, gkey_index, false);
    unsafe {
        libc::close(fd);
    }
    println!("\n(fin de la surveillance)");
}

fn print_usage() {
    println!("logi-headset — remap mute + LED + batterie pour casque Logitech (HID++ natif)\n");
    println!("Usage: logi-headset [OPTIONS]\n");
    println!("Config: ~/.config/logi-headset/config  (lignes cle=valeur)");
    println!("  key        = playpause|next|previous|stop|mute|micmute|volumeup|volumedown|<code>");
    println!("  key_double = (optionnel) action du double-clic, memes valeurs que key, ou 'none'");
    println!("  double_ms  = fenetre du double-clic en ms (defaut 1000, 100..3000)");
    println!("  leds       = keep|off|color");
    println!("  led_color  = RRGGBB   (utilise si leds=color)");
    println!("  battery_warn = seuil d'alerte batterie en % (defaut 15, 'off' pour desactiver)\n");
    println!("Options:");
    println!("  --config <chemin>   Utilise un autre fichier de config");
    println!("  --leds-off          Force l'extinction des LED (override la config)");
    println!("  --diagnose          Enumere les features HID++ du casque (service arrete)");
    println!("  --watch             Affiche les rapports HID++ bruts en direct (debug)");
    println!("  -h, --help          Affiche cette aide");
}

fn main() {
    let mut cfg_path = default_config_path();
    let mut force_leds_off = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--leds-off" => force_leds_off = true,
            "--diagnose" => {
                diagnose();
                return;
            }
            "--watch" => {
                watch();
                return;
            }
            "--config" => match args.next() {
                Some(p) => cfg_path = std::path::PathBuf::from(p),
                None => {
                    eprintln!("--config attend un chemin");
                    std::process::exit(2);
                }
            },
            "-h" | "--help" => {
                print_usage();
                return;
            }
            other => {
                eprintln!("option inconnue: {other}\n");
                print_usage();
                std::process::exit(2);
            }
        }
    }
    let mut cfg = load_config(&cfg_path);
    if force_leds_off {
        cfg.led_mode = LedMode::Off;
    }
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        // notify-send est lance en fire-and-forget : SIG_IGN -> le noyau recolte les
        // enfants automatiquement (pas de zombies sur ce demon de longue duree).
        libc::signal(libc::SIGCHLD, libc::SIG_IGN);
    }
    let mut keys = vec![cfg.key_code];
    if let Some(dc) = cfg.double_code {
        if dc != cfg.key_code {
            keys.push(dc);
        }
    }
    // /dev/uinput peut ne pas etre pret tout de suite au boot : on reessaie au
    // lieu de mourir (evite l'echec-puis-restart aleatoire du service).
    let ui_fd = loop {
        match open_uinput(&keys) {
            Ok(fd) => break fd,
            Err(e) => {
                if stopped() {
                    return;
                }
                log!("uinput indisponible ({e}) — nouvelle tentative dans 2 s");
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    };
    let batt_desc = if cfg.battery_warn == 0 {
        "sans alerte".to_string()
    } else {
        format!("< {} %", cfg.battery_warn)
    };
    log!(
        "demarre (touche={}, double={:?} fenetre={}ms, leds={:?}, batterie={}) — config: {}",
        cfg.key_code,
        cfg.double_code,
        cfg.double_ms,
        cfg.led_mode,
        batt_desc,
        cfg_path.display()
    );
    set_status("recherche");
    while !stopped() {
        serve(ui_fd, &cfg);
        if !stopped() {
            std::thread::sleep(Duration::from_secs(2)); // attente avant de re-chercher le casque
        }
    }
    set_status("arrete");
    close_uinput(ui_fd);
    log!("arret.");
}
