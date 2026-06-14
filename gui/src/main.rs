// logi-headset-config — configuration panel for the G733 remap (GTK4).
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

//! Panneau de configuration du remap G733 (GTK4 / gtk-rs).
//!
//! Lit / ecrit le meme fichier de config que le demon
//! (~/.config/logi-headset/config) et pilote le service utilisateur via
//! `systemctl --user`. Affiche un statut live (demon actif, casque connecte)
//! lu dans le fichier d'etat publie par le demon.
//!
//! Interface multilingue : la langue est deduite de la locale systeme (LANG /
//! LC_MESSAGES), avec repli sur l'anglais. Langues : en, fr, de, it, es, pt.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use adw::prelude::*;
use adw::{Application, ApplicationWindow, HeaderBar, ToolbarView};
use gtk::{
    glib, AboutDialog, Box as GtkBox, Button, ColorDialog, ColorDialogButton, DropDown, Frame,
    Grid, Label, License, Orientation, ScrolledWindow, SpinButton, Switch, TextView, Window,
};

const APP_ID: &str = "io.github.whitewolf832.LogiHeadset";
const SERVICE: &str = "logi-headset.service";
// Doit rester aligne sur ISSUES_URL du demon. PLACEHOLDER jusqu'a la creation du depot.
const ISSUES_URL: &str = "https://github.com/WhiteWolf832/logi-headset/issues";

// Valeurs de config (jamais traduites) ; les libelles viennent de `Strings`.
const KEY_VALUES: [&str; 8] = [
    "playpause", "next", "previous", "stop", "mute", "micmute", "volumeup", "volumedown",
];
const LED_VALUES: [&str; 3] = ["keep", "off", "color"];

const DEFAULT_BATTERY_WARN: u8 = 15; // aligne sur le defaut du demon

// ---- i18n -------------------------------------------------------------------
/// Tous les libelles de l'interface, une instance par langue. Le compilateur
/// garantit que chaque langue remplit chaque champ (rien d'oublie).
struct Strings {
    window_title: &'static str,
    sec_single: &'static str,
    sec_double: &'static str,
    sec_led: &'static str,
    sec_battery: &'static str,
    sec_service: &'static str,
    sec_status: &'static str,
    keys: [&'static str; 8], // memes positions que KEY_VALUES
    leds: [&'static str; 3], // memes positions que LED_VALUES
    dbl_none: &'static str,
    ms_label: &'static str,
    dbl_note: &'static str,
    led_note: &'static str,
    batt_enable: &'static str,
    batt_threshold: &'static str,
    batt_note: &'static str,
    svc_autostart: &'static str,
    btn_start: &'static str,
    btn_stop: &'static str,
    btn_apply: &'static str,
    daemon_active: &'static str,   // markup Pango
    daemon_stopped: &'static str,  // markup Pango
    headset_connected: &'static str,
    headset_searching: &'static str,
    headset_none: &'static str,
    status_battery: &'static str, // prefixe de la ligne charge dans Statut
    info_applied: &'static str,
    info_restart_failed: &'static str,
    info_saved: &'static str,
    info_write_err: &'static str, // prefixe ; l'erreur est ajoutee a la suite
    btn_analyze: &'static str,
    analyze_title: &'static str,
    analyze_copy: &'static str,
    analyze_open: &'static str,
    analyze_close: &'static str,
    analyze_fail: &'static str,
    g533_note: &'static str, // affichee quand un modele a reveil lent (G533) est detecte
    led_none: &'static str,  // affichee quand le casque n'a pas de LED (config LED grisee)
    btn_about: &'static str,
}

const EN: Strings = Strings {
    window_title: "Logitech Headset",
    sec_single: "Single click",
    sec_double: "Double click",
    sec_led: "Headset LEDs",
    sec_battery: "Battery",
    sec_service: "Service",
    sec_status: "Status",
    keys: [
        "Play / Pause",
        "Next track",
        "Previous track",
        "Stop",
        "Mute",
        "Mute microphone",
        "Volume up",
        "Volume down",
    ],
    leds: ["Leave unchanged", "Turn off", "Solid color"],
    dbl_none: "None",
    ms_label: "Delay before single click (ms)",
    dbl_note: "Double click adds this delay to the single click (time to detect a 2nd press).",
    led_note: "Not all colors are rendered accurately by the headset.",
    batt_enable: "Warn when battery is low",
    batt_threshold: "Alert threshold (%)",
    batt_note: "The headset only reports a voltage: the percentage is estimated.",
    svc_autostart: "Start on login",
    btn_start: "Start",
    btn_stop: "Stop",
    btn_apply: "Apply",
    daemon_active: "Daemon: <b>active</b>",
    daemon_stopped: "Daemon: <b>stopped</b>",
    headset_connected: "Headset: connected ✓",
    headset_searching: "Headset: not detected",
    headset_none: "Headset: —",
    status_battery: "Battery: ",
    info_applied: "Configuration applied (service restarted).",
    info_restart_failed: "Config saved, but restarting the service failed.",
    info_saved: "Configuration saved (applied on next start).",
    info_write_err: "Write error: ",
    btn_analyze: "Analyze headset",
    analyze_title: "Headset diagnostic",
    analyze_copy: "Copy",
    analyze_open: "Open issues",
    analyze_close: "Close",
    analyze_fail: "Analysis failed — is the headset connected and on?",
    g533_note: "⏱ After power-on, this headset's mute button takes about a minute to respond.",
    led_none: "This headset has no LEDs.",
    btn_about: "About…",
};

const FR: Strings = Strings {
    window_title: "Logitech Headset",
    sec_single: "Clic simple",
    sec_double: "Double-clic",
    sec_led: "LED du casque",
    sec_battery: "Batterie",
    sec_service: "Service",
    sec_status: "Statut",
    keys: [
        "Lecture / Pause",
        "Piste suivante",
        "Piste précédente",
        "Stop",
        "Muet",
        "Couper le micro",
        "Volume +",
        "Volume −",
    ],
    leds: ["Ne pas toucher", "Éteindre", "Couleur fixe"],
    dbl_none: "Aucune",
    ms_label: "Délai avant le clic simple (ms)",
    dbl_note: "Le double-clic ajoute ce délai au clic simple (le temps de détecter un 2e appui).",
    led_note: "Toutes les couleurs ne sont pas rendues fidèlement par le casque.",
    batt_enable: "Avertir quand la batterie est faible",
    batt_threshold: "Seuil d'alerte (%)",
    batt_note: "Le casque ne fournit qu'une tension : le pourcentage est estimé.",
    svc_autostart: "Démarrer à la connexion",
    btn_start: "Démarrer",
    btn_stop: "Arrêter",
    btn_apply: "Appliquer",
    daemon_active: "Démon : <b>actif</b>",
    daemon_stopped: "Démon : <b>arrêté</b>",
    headset_connected: "Casque : connecté ✓",
    headset_searching: "Casque : non détecté",
    headset_none: "Casque : —",
    status_battery: "Batterie : ",
    info_applied: "Configuration appliquée (service redémarré).",
    info_restart_failed: "Config enregistrée, mais le redémarrage du service a échoué.",
    info_saved: "Configuration enregistrée (prise en compte au démarrage).",
    info_write_err: "Erreur d'écriture : ",
    btn_analyze: "Analyser le casque",
    analyze_title: "Diagnostic du casque",
    analyze_copy: "Copier",
    analyze_open: "Ouvrir les issues",
    analyze_close: "Fermer",
    analyze_fail: "Analyse impossible — casque connecté et allumé ?",
    g533_note: "⏱ Après allumage, le bouton mute de ce casque met ~1 min à répondre.",
    led_none: "Ce casque n'a pas de LED.",
    btn_about: "À propos…",
};

const DE: Strings = Strings {
    window_title: "Logitech Headset",
    sec_single: "Einfacher Klick",
    sec_double: "Doppelklick",
    sec_led: "Headset-LEDs",
    sec_battery: "Akku",
    sec_service: "Dienst",
    sec_status: "Status",
    keys: [
        "Wiedergabe / Pause",
        "Nächster Titel",
        "Vorheriger Titel",
        "Stopp",
        "Stumm",
        "Mikrofon stumm",
        "Lauter",
        "Leiser",
    ],
    leds: ["Nicht ändern", "Ausschalten", "Feste Farbe"],
    dbl_none: "Keine",
    ms_label: "Verzögerung vor Einzelklick (ms)",
    dbl_note: "Der Doppelklick fügt dem Einzelklick diese Verzögerung hinzu (Zeit zum Erkennen eines 2. Drucks).",
    led_note: "Nicht alle Farben werden vom Headset originalgetreu wiedergegeben.",
    batt_enable: "Warnen, wenn der Akku schwach ist",
    batt_threshold: "Warnschwelle (%)",
    batt_note: "Das Headset liefert nur eine Spannung: der Prozentwert ist geschätzt.",
    svc_autostart: "Beim Anmelden starten",
    btn_start: "Starten",
    btn_stop: "Stoppen",
    btn_apply: "Übernehmen",
    daemon_active: "Dienst: <b>aktiv</b>",
    daemon_stopped: "Dienst: <b>gestoppt</b>",
    headset_connected: "Headset: verbunden ✓",
    headset_searching: "Headset: nicht erkannt",
    headset_none: "Headset: —",
    status_battery: "Akku: ",
    info_applied: "Konfiguration übernommen (Dienst neu gestartet).",
    info_restart_failed: "Konfiguration gespeichert, aber der Neustart des Dienstes ist fehlgeschlagen.",
    info_saved: "Konfiguration gespeichert (wird beim nächsten Start übernommen).",
    info_write_err: "Schreibfehler: ",
    btn_analyze: "Headset analysieren",
    analyze_title: "Headset-Diagnose",
    analyze_copy: "Kopieren",
    analyze_open: "Issues öffnen",
    analyze_close: "Schließen",
    analyze_fail: "Analyse fehlgeschlagen — Headset verbunden und an?",
    g533_note: "⏱ Nach dem Einschalten reagiert die Stummtaste dieses Headsets erst nach ~1 Min.",
    led_none: "Dieses Headset hat keine LEDs.",
    btn_about: "Über…",
};

const IT: Strings = Strings {
    window_title: "Logitech Headset",
    sec_single: "Clic singolo",
    sec_double: "Doppio clic",
    sec_led: "LED delle cuffie",
    sec_battery: "Batteria",
    sec_service: "Servizio",
    sec_status: "Stato",
    keys: [
        "Riproduci / Pausa",
        "Traccia successiva",
        "Traccia precedente",
        "Stop",
        "Muto",
        "Disattiva microfono",
        "Volume su",
        "Volume giù",
    ],
    leds: ["Non toccare", "Spegnere", "Colore fisso"],
    dbl_none: "Nessuna",
    ms_label: "Ritardo prima del clic singolo (ms)",
    dbl_note: "Il doppio clic aggiunge questo ritardo al clic singolo (il tempo per rilevare una 2ª pressione).",
    led_note: "Non tutti i colori vengono riprodotti fedelmente dalle cuffie.",
    batt_enable: "Avvisa quando la batteria è scarica",
    batt_threshold: "Soglia di avviso (%)",
    batt_note: "Le cuffie forniscono solo una tensione: la percentuale è stimata.",
    svc_autostart: "Avvia all'accesso",
    btn_start: "Avvia",
    btn_stop: "Ferma",
    btn_apply: "Applica",
    daemon_active: "Demone: <b>attivo</b>",
    daemon_stopped: "Demone: <b>fermo</b>",
    headset_connected: "Cuffie: connesse ✓",
    headset_searching: "Cuffie: non rilevate",
    headset_none: "Cuffie: —",
    status_battery: "Batteria: ",
    info_applied: "Configurazione applicata (servizio riavviato).",
    info_restart_failed: "Configurazione salvata, ma il riavvio del servizio non è riuscito.",
    info_saved: "Configurazione salvata (applicata al prossimo avvio).",
    info_write_err: "Errore di scrittura: ",
    btn_analyze: "Analizza le cuffie",
    analyze_title: "Diagnostica cuffie",
    analyze_copy: "Copia",
    analyze_open: "Apri le issue",
    analyze_close: "Chiudi",
    analyze_fail: "Analisi non riuscita — cuffie collegate e accese?",
    g533_note: "⏱ Dopo l'accensione, il tasto muto di queste cuffie risponde dopo ~1 min.",
    led_none: "Queste cuffie non hanno LED.",
    btn_about: "Informazioni…",
};

const ES: Strings = Strings {
    window_title: "Logitech Headset",
    sec_single: "Clic simple",
    sec_double: "Doble clic",
    sec_led: "LED de los cascos",
    sec_battery: "Batería",
    sec_service: "Servicio",
    sec_status: "Estado",
    keys: [
        "Reproducir / Pausa",
        "Pista siguiente",
        "Pista anterior",
        "Detener",
        "Silenciar",
        "Silenciar micrófono",
        "Subir volumen",
        "Bajar volumen",
    ],
    leds: ["No tocar", "Apagar", "Color fijo"],
    dbl_none: "Ninguna",
    ms_label: "Retraso antes del clic simple (ms)",
    dbl_note: "El doble clic añade este retraso al clic simple (tiempo para detectar una 2ª pulsación).",
    led_note: "El casco no reproduce fielmente todos los colores.",
    batt_enable: "Avisar cuando la batería esté baja",
    batt_threshold: "Umbral de aviso (%)",
    batt_note: "El casco solo proporciona una tensión: el porcentaje es estimado.",
    svc_autostart: "Iniciar al iniciar sesión",
    btn_start: "Iniciar",
    btn_stop: "Detener",
    btn_apply: "Aplicar",
    daemon_active: "Demonio: <b>activo</b>",
    daemon_stopped: "Demonio: <b>detenido</b>",
    headset_connected: "Casco: conectado ✓",
    headset_searching: "Casco: no detectado",
    headset_none: "Casco: —",
    status_battery: "Batería: ",
    info_applied: "Configuración aplicada (servicio reiniciado).",
    info_restart_failed: "Configuración guardada, pero el reinicio del servicio falló.",
    info_saved: "Configuración guardada (se aplicará al iniciar).",
    info_write_err: "Error de escritura: ",
    btn_analyze: "Analizar el casco",
    analyze_title: "Diagnóstico del casco",
    analyze_copy: "Copiar",
    analyze_open: "Abrir issues",
    analyze_close: "Cerrar",
    analyze_fail: "Análisis fallido — ¿casco conectado y encendido?",
    g533_note: "⏱ Tras encender, el botón de silencio de este casco tarda ~1 min en responder.",
    led_none: "Este casco no tiene LED.",
    btn_about: "Acerca de…",
};

const PT: Strings = Strings {
    window_title: "Logitech Headset",
    sec_single: "Clique simples",
    sec_double: "Clique duplo",
    sec_led: "LEDs do headset",
    sec_battery: "Bateria",
    sec_service: "Serviço",
    sec_status: "Estado",
    keys: [
        "Reproduzir / Pausa",
        "Faixa seguinte",
        "Faixa anterior",
        "Parar",
        "Sem som",
        "Silenciar microfone",
        "Aumentar volume",
        "Diminuir volume",
    ],
    leds: ["Não alterar", "Desligar", "Cor fixa"],
    dbl_none: "Nenhuma",
    ms_label: "Atraso antes do clique simples (ms)",
    dbl_note: "O clique duplo adiciona este atraso ao clique simples (tempo para detetar uma 2ª pressão).",
    led_note: "Nem todas as cores são reproduzidas fielmente pelo headset.",
    batt_enable: "Avisar quando a bateria estiver fraca",
    batt_threshold: "Limiar de aviso (%)",
    batt_note: "O headset só fornece uma tensão: a percentagem é estimada.",
    svc_autostart: "Iniciar ao iniciar sessão",
    btn_start: "Iniciar",
    btn_stop: "Parar",
    btn_apply: "Aplicar",
    daemon_active: "Serviço: <b>ativo</b>",
    daemon_stopped: "Serviço: <b>parado</b>",
    headset_connected: "Headset: ligado ✓",
    headset_searching: "Headset: não detetado",
    headset_none: "Headset: —",
    status_battery: "Bateria: ",
    info_applied: "Configuração aplicada (serviço reiniciado).",
    info_restart_failed: "Configuração guardada, mas o reinício do serviço falhou.",
    info_saved: "Configuração guardada (aplicada no próximo arranque).",
    info_write_err: "Erro de escrita: ",
    btn_analyze: "Analisar o headset",
    analyze_title: "Diagnóstico do headset",
    analyze_copy: "Copiar",
    analyze_open: "Abrir issues",
    analyze_close: "Fechar",
    analyze_fail: "Análise falhou — headset ligado e aceso?",
    g533_note: "⏱ Depois de ligar, o botão de silêncio deste headset demora ~1 min a responder.",
    led_none: "Este headset não tem LEDs.",
    btn_about: "Acerca de…",
};

/// Langue de l'interface, deduite de la locale systeme (repli : anglais).
fn strings() -> &'static Strings {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(var) {
            match v.get(0..2) {
                Some("fr") => return &FR,
                Some("de") => return &DE,
                Some("it") => return &IT,
                Some("es") => return &ES,
                Some("pt") => return &PT,
                Some("en") => return &EN,
                _ => {}
            }
        }
    }
    &EN
}

// ---- config & etat ----------------------------------------------------------
fn config_path() -> PathBuf {
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            PathBuf::from(home).join(".config")
        });
    dir.join("logi-headset").join("config")
}

fn status_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("logi-headset.status")
}

fn battery_status_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("logi-headset.battery")
}

fn device_status_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("logi-headset.device")
}

fn caps_status_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("logi-headset.caps")
}

struct Cfg {
    key: String,
    key_double: String,
    double_ms: f64,
    leds: String,
    color: String,
    battery_warn: u8, // 0 = avertissement desactive
}

fn load_cfg() -> Cfg {
    let mut cfg = Cfg {
        key: "playpause".to_string(),
        key_double: "none".to_string(),
        double_ms: 1000.0,
        leds: "keep".to_string(),
        color: "ffffff".to_string(),
        battery_warn: DEFAULT_BATTERY_WARN,
    };
    if let Ok(txt) = fs::read_to_string(config_path()) {
        for line in txt.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let (k, v) = (k.trim(), v.trim());
                match k {
                    "key" => cfg.key = v.to_string(),
                    "key_double" => cfg.key_double = v.to_string(),
                    "double_ms" => {
                        if let Ok(n) = v.parse::<f64>() {
                            cfg.double_ms = n.clamp(100.0, 3000.0);
                        }
                    }
                    "leds" => cfg.leds = v.to_string(),
                    "led_color" => cfg.color = v.trim_start_matches('#').to_string(),
                    "battery_warn" | "batterie" => {
                        cfg.battery_warn = match v.to_lowercase().as_str() {
                            "off" | "none" | "aucune" | "non" => 0,
                            s => s.parse::<u8>().unwrap_or(DEFAULT_BATTERY_WARN).min(100),
                        };
                    }
                    _ => {}
                }
            }
        }
    }
    cfg
}

#[allow(clippy::too_many_arguments)]
fn save_cfg(
    key: &str,
    key_double: &str,
    double_ms: u64,
    leds: &str,
    color: &str,
    battery_warn: u8,
) -> std::io::Result<()> {
    let p = config_path();
    if let Some(dir) = p.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(
        p,
        format!(
            "key={key}\nkey_double={key_double}\ndouble_ms={double_ms}\nleds={leds}\nled_color={color}\nbattery_warn={battery_warn}\n"
        ),
    )
}

fn read_status() -> Option<String> {
    fs::read_to_string(status_path())
        .ok()
        .map(|s| s.trim().to_string())
}

/// Charge batterie publiee par le demon (libelle pret a afficher), si presente.
fn read_battery_status() -> Option<String> {
    let v = fs::read_to_string(battery_status_path()).ok()?;
    let v = v.trim();
    if v.is_empty() {
        None
    } else {
        Some(v.to_string())
    }
}

/// Nom de modele publie par le demon (ex. « G533 Gaming Headset »), si present.
fn read_device_status() -> Option<String> {
    let v = fs::read_to_string(device_status_path()).ok()?;
    let v = v.trim();
    if v.is_empty() {
        None
    } else {
        Some(v.to_string())
    }
}

/// Capacite RGB publiee par le demon : Some(true/false), ou None si inconnue.
fn read_has_rgb() -> Option<bool> {
    let txt = fs::read_to_string(caps_status_path()).ok()?;
    txt.lines()
        .find_map(|l| l.trim().strip_prefix("rgb=").map(|v| v.trim() == "1"))
}

// ---- service systemd (utilisateur) ------------------------------------------
fn systemctl(action: &str) -> bool {
    Command::new("systemctl")
        .args(["--user", action, SERVICE])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn service_is(query: &str) -> bool {
    Command::new("systemctl")
        .args(["--user", query, "--quiet", SERVICE])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Lance `logi-headset --diagnose`, en arretant/redemarrant le service au besoin
/// (les deux process ne peuvent pas tenir le hidraw en meme temps). Restaure l'etat
/// initial du service. None si le casque n'a pas repondu.
fn run_diagnose() -> Option<String> {
    let was_active = service_is("is-active");
    if was_active {
        systemctl("stop");
    }
    let out = Command::new("logi-headset").arg("--diagnose").output();
    if was_active {
        systemctl("start");
    }
    let text = String::from_utf8_lossy(&out.ok()?.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Fenetre affichant le rapport de diagnostic : Copier / Ouvrir les issues / Fermer.
fn show_diagnostic_dialog(parent: &ApplicationWindow, s: &'static Strings, report: &str) {
    let dlg = Window::builder()
        .transient_for(parent)
        .modal(true)
        .title(s.analyze_title)
        .default_width(600)
        .default_height(460)
        .build();

    let tv = TextView::new();
    tv.set_editable(false);
    tv.set_monospace(true);
    tv.set_left_margin(8);
    tv.set_top_margin(8);
    tv.buffer().set_text(report);
    let scroll = ScrolledWindow::builder().child(&tv).vexpand(true).build();

    let copy_btn = Button::with_label(s.analyze_copy);
    let open_btn = Button::with_label(s.analyze_open);
    let close_btn = Button::with_label(s.analyze_close);
    close_btn.add_css_class("suggested-action");
    {
        let report = report.to_string();
        copy_btn.connect_clicked(move |btn| {
            btn.clipboard().set_text(&report);
        });
    }
    open_btn.connect_clicked(|_| {
        let _ = gtk::gio::AppInfo::launch_default_for_uri(ISSUES_URL, gtk::gio::AppLaunchContext::NONE);
    });
    {
        let dlg = dlg.clone();
        close_btn.connect_clicked(move |_| dlg.close());
    }

    let btn_row = GtkBox::new(Orientation::Horizontal, 8);
    btn_row.append(&copy_btn);
    btn_row.append(&open_btn);
    let spacer = GtkBox::new(Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    btn_row.append(&spacer);
    btn_row.append(&close_btn);

    let vbox = GtkBox::new(Orientation::Vertical, 8);
    vbox.set_margin_top(12);
    vbox.set_margin_bottom(12);
    vbox.set_margin_start(12);
    vbox.set_margin_end(12);
    vbox.append(&scroll);
    vbox.append(&btn_row);
    dlg.set_child(Some(&vbox));
    dlg.present();
}

// ---- couleurs ----------------------------------------------------------------
fn rgba_to_hex(c: &gtk::gdk::RGBA) -> String {
    let f = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("{:02x}{:02x}{:02x}", f(c.red()), f(c.green()), f(c.blue()))
}

fn hex_to_rgba(hex: &str) -> gtk::gdk::RGBA {
    format!("#{hex}")
        .parse()
        .or_else(|_| "#ffffff".parse())
        .unwrap()
}

fn index_of(values: &[&str], value: &str) -> u32 {
    values.iter().position(|v| *v == value).unwrap_or(0) as u32
}

// ---- UI ----------------------------------------------------------------------
fn section(title: &str, child: &impl IsA<gtk::Widget>) -> Frame {
    let frame = Frame::new(Some(title));
    frame.set_child(Some(child));
    frame.set_valign(gtk::Align::Start); // colle en haut de sa cellule de grille
    frame.set_hexpand(true);
    frame
}

/// Petite ligne "label ........ widget" (le label pousse le widget a droite).
fn labeled_row(text: &str, widget: &impl IsA<gtk::Widget>) -> GtkBox {
    let row = GtkBox::new(Orientation::Horizontal, 8);
    let label = Label::new(Some(text));
    label.set_halign(gtk::Align::Start);
    row.append(&label);
    let spacer = GtkBox::new(Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    row.append(&spacer);
    row.append(widget);
    row
}

/// Fenetre « A propos » : nom, version, copyright et licence GPLv3 (dialogue GTK natif).
fn show_about(parent: &ApplicationWindow) {
    let about = AboutDialog::builder()
        .transient_for(parent)
        .modal(true)
        .program_name("logi-headset")
        .version(env!("CARGO_PKG_VERSION"))
        .comments("Mute-button remap, LEDs & battery for Logitech wireless headsets (HID++)")
        .copyright("Copyright © 2026 WhiteWolf832")
        .license_type(License::Gpl30)
        .website("https://github.com/WhiteWolf832/logi-headset")
        .website_label("github.com/WhiteWolf832/logi-headset")
        .authors(["WhiteWolf832", "Velsa"])
        .build();
    about.present();
}

fn build_ui(app: &Application) {
    let s = strings();
    let cfg = load_cfg();

    // --- Touche ---
    let key_dd = DropDown::from_strings(&s.keys);
    key_dd.set_selected(index_of(&KEY_VALUES, &cfg.key));
    key_dd.set_margin_top(8);
    key_dd.set_margin_bottom(8);
    key_dd.set_margin_start(8);
    key_dd.set_margin_end(8);

    // --- Double-clic ---
    let mut dbl_labels: Vec<&str> = vec![s.dbl_none];
    dbl_labels.extend(s.keys.iter().copied());
    let dbl_dd = DropDown::from_strings(&dbl_labels);
    let dbl_active = !(cfg.key_double == "none" || cfg.key_double.is_empty());
    let dbl_sel = if dbl_active {
        index_of(&KEY_VALUES, &cfg.key_double) + 1
    } else {
        0
    };
    dbl_dd.set_selected(dbl_sel);
    dbl_dd.set_margin_top(8);
    dbl_dd.set_margin_start(8);
    dbl_dd.set_margin_end(8);

    // Reglage du delai (double_ms), grise quand "Aucune"
    let ms_spin = SpinButton::with_range(100.0, 3000.0, 50.0);
    ms_spin.set_digits(0);
    ms_spin.set_value(cfg.double_ms);
    ms_spin.set_sensitive(dbl_active);
    let ms_row = labeled_row(s.ms_label, &ms_spin);
    ms_row.set_margin_start(8);
    ms_row.set_margin_end(8);
    {
        let ms_spin = ms_spin.clone();
        dbl_dd.connect_selected_notify(move |dd| {
            ms_spin.set_sensitive(dd.selected() != 0);
        });
    }

    let dbl_note = Label::new(Some(s.dbl_note));
    dbl_note.set_halign(gtk::Align::Start);
    dbl_note.set_wrap(true);
    dbl_note.add_css_class("dim-label");
    dbl_note.set_margin_start(8);
    dbl_note.set_margin_end(8);
    dbl_note.set_margin_bottom(8);
    let dbl_box = GtkBox::new(Orientation::Vertical, 4);
    dbl_box.append(&dbl_dd);
    dbl_box.append(&ms_row);
    dbl_box.append(&dbl_note);

    // --- LED ---
    let led_dd = DropDown::from_strings(&s.leds);
    led_dd.set_selected(index_of(&LED_VALUES, &cfg.leds));
    let color_btn = ColorDialogButton::builder()
        .dialog(&ColorDialog::new())
        .build();
    color_btn.set_rgba(&hex_to_rgba(&cfg.color));
    color_btn.set_sensitive(cfg.leds == "color");
    let led_row = GtkBox::new(Orientation::Horizontal, 8);
    led_dd.set_hexpand(true);
    led_row.append(&led_dd);
    led_row.append(&color_btn);
    {
        let color_btn = color_btn.clone();
        led_dd.connect_selected_notify(move |dd| {
            let is_color = dd.selected() as usize == index_of(&LED_VALUES, "color") as usize;
            color_btn.set_sensitive(is_color);
        });
    }
    let led_note = Label::new(Some(s.led_note));
    led_note.set_halign(gtk::Align::Start);
    led_note.set_wrap(true);
    led_note.add_css_class("dim-label");
    let led_box = GtkBox::new(Orientation::Vertical, 4);
    led_box.set_margin_top(8);
    led_box.set_margin_bottom(8);
    led_box.set_margin_start(8);
    led_box.set_margin_end(8);
    led_box.append(&led_row);
    led_box.append(&led_note);

    // --- Batterie ---
    let batt_switch = Switch::new();
    batt_switch.set_active(cfg.battery_warn > 0);
    batt_switch.set_valign(gtk::Align::Center);
    let batt_spin = SpinButton::with_range(1.0, 100.0, 5.0);
    batt_spin.set_digits(0);
    batt_spin.set_value(if cfg.battery_warn > 0 {
        cfg.battery_warn as f64
    } else {
        DEFAULT_BATTERY_WARN as f64
    });
    batt_spin.set_sensitive(cfg.battery_warn > 0);
    {
        let batt_spin = batt_spin.clone();
        batt_switch.connect_active_notify(move |sw| {
            batt_spin.set_sensitive(sw.is_active());
        });
    }
    let batt_note = Label::new(Some(s.batt_note));
    batt_note.set_halign(gtk::Align::Start);
    batt_note.set_wrap(true);
    batt_note.add_css_class("dim-label");
    let batt_box = GtkBox::new(Orientation::Vertical, 4);
    batt_box.set_margin_top(8);
    batt_box.set_margin_bottom(8);
    batt_box.set_margin_start(8);
    batt_box.set_margin_end(8);
    batt_box.append(&labeled_row(s.batt_enable, &batt_switch));
    batt_box.append(&labeled_row(s.batt_threshold, &batt_spin));
    batt_box.append(&batt_note);

    // --- Service ---
    let enable_switch = Switch::new();
    enable_switch.set_active(service_is("is-enabled"));
    enable_switch.set_valign(gtk::Align::Center);
    enable_switch.connect_active_notify(|sw| {
        systemctl(if sw.is_active() { "enable" } else { "disable" });
    });

    let start_btn = Button::with_label(s.btn_start);
    let stop_btn = Button::with_label(s.btn_stop);
    start_btn.set_hexpand(true);
    stop_btn.set_hexpand(true);
    start_btn.connect_clicked(|_| {
        systemctl("start");
    });
    stop_btn.connect_clicked(|_| {
        systemctl("stop");
    });
    let btn_row = GtkBox::new(Orientation::Horizontal, 8);
    btn_row.append(&start_btn);
    btn_row.append(&stop_btn);

    let service_box = GtkBox::new(Orientation::Vertical, 8);
    service_box.set_margin_top(8);
    service_box.set_margin_bottom(8);
    service_box.set_margin_start(8);
    service_box.set_margin_end(8);
    service_box.append(&labeled_row(s.svc_autostart, &enable_switch));
    service_box.append(&btn_row);

    // --- Statut ---
    let lbl_service = Label::new(None);
    lbl_service.set_halign(gtk::Align::Start);
    let lbl_headset = Label::new(None);
    lbl_headset.set_halign(gtk::Align::Start);
    let lbl_device = Label::new(None); // nom de modele (gras), vide si non connecte
    lbl_device.set_halign(gtk::Align::Start);
    let lbl_battery = Label::new(None);
    lbl_battery.set_halign(gtk::Align::Start);
    let lbl_g533 = Label::new(None); // note "delai mute" affichee si modele a reveil lent
    lbl_g533.set_halign(gtk::Align::Start);
    lbl_g533.set_wrap(true);
    lbl_g533.add_css_class("dim-label");
    let status_box = GtkBox::new(Orientation::Vertical, 4);
    status_box.set_margin_top(8);
    status_box.set_margin_bottom(8);
    status_box.set_margin_start(8);
    status_box.set_margin_end(8);
    status_box.append(&lbl_service);
    status_box.append(&lbl_headset);
    status_box.append(&lbl_device);
    status_box.append(&lbl_battery);
    status_box.append(&lbl_g533);
    let analyze_btn = Button::with_label(s.btn_analyze);
    analyze_btn.set_margin_top(6);
    status_box.append(&analyze_btn);

    // --- Appliquer + info ---
    let info = Label::new(None);
    info.set_halign(gtk::Align::Start);
    info.set_wrap(true);
    let apply = Button::with_label(s.btn_apply);
    apply.add_css_class("suggested-action");
    apply.set_halign(gtk::Align::End);
    let about_btn = Button::with_label(s.btn_about);
    {
        let key_dd = key_dd.clone();
        let dbl_dd = dbl_dd.clone();
        let ms_spin = ms_spin.clone();
        let led_dd = led_dd.clone();
        let color_btn = color_btn.clone();
        let batt_switch = batt_switch.clone();
        let batt_spin = batt_spin.clone();
        let info = info.clone();
        apply.connect_clicked(move |_| {
            let key = KEY_VALUES
                .get(key_dd.selected() as usize)
                .copied()
                .unwrap_or("playpause");
            let dsel = dbl_dd.selected() as usize;
            let key_double = if dsel == 0 {
                "none"
            } else {
                KEY_VALUES.get(dsel - 1).copied().unwrap_or("none")
            };
            let double_ms = ms_spin.value().round() as u64;
            let leds = LED_VALUES
                .get(led_dd.selected() as usize)
                .copied()
                .unwrap_or("keep");
            let color = rgba_to_hex(&color_btn.rgba());
            let battery_warn = if batt_switch.is_active() {
                batt_spin.value().round() as u8
            } else {
                0
            };
            match save_cfg(key, key_double, double_ms, leds, &color, battery_warn) {
                Ok(_) => {
                    if service_is("is-active") {
                        let ok = systemctl("restart");
                        info.set_text(if ok {
                            s.info_applied
                        } else {
                            s.info_restart_failed
                        });
                    } else {
                        info.set_text(s.info_saved);
                    }
                }
                Err(e) => info.set_text(&format!("{}{e}", s.info_write_err)),
            }
        });
    }

    // --- Assemblage : deux colonnes pour une fenetre plus compacte ---
    let root = Grid::new();
    root.set_row_spacing(12);
    root.set_column_spacing(12);
    root.set_column_homogeneous(true);
    root.set_margin_top(16);
    root.set_margin_bottom(16);
    root.set_margin_start(16);
    root.set_margin_end(16);
    // Colonne gauche
    root.attach(&section(s.sec_single, &key_dd), 0, 0, 1, 1);
    root.attach(&section(s.sec_double, &dbl_box), 0, 1, 1, 1);
    root.attach(&section(s.sec_status, &status_box), 0, 2, 1, 1);
    // Colonne droite
    root.attach(&section(s.sec_led, &led_box), 1, 0, 1, 1);
    root.attach(&section(s.sec_battery, &batt_box), 1, 1, 1, 1);
    root.attach(&section(s.sec_service, &service_box), 1, 2, 1, 1);
    // Bas, pleine largeur (les deux colonnes)
    // Barre d'action : « À propos » a gauche, « Appliquer » a droite.
    let action_row = GtkBox::new(Orientation::Horizontal, 8);
    action_row.append(&about_btn);
    let action_spacer = GtkBox::new(Orientation::Horizontal, 0);
    action_spacer.set_hexpand(true);
    action_row.append(&action_spacer);
    action_row.append(&apply);
    root.attach(&action_row, 0, 3, 2, 1);
    root.attach(&info, 0, 4, 2, 1);

    // --- Rafraichissement du statut (1 s) ---
    let update = move || {
        let active = service_is("is-active");
        lbl_service.set_markup(if active {
            s.daemon_active
        } else {
            s.daemon_stopped
        });
        let status = if active { read_status() } else { None };
        let connected = status.as_deref() == Some("connecte");
        lbl_headset.set_text(match status.as_deref() {
            Some("connecte") => s.headset_connected,
            Some("recherche") => s.headset_searching,
            _ => s.headset_none,
        });
        // Modele (en gras) — seulement si connecte et publie par le demon.
        let model = if connected { read_device_status() } else { None };
        match &model {
            Some(name) => {
                lbl_device.set_markup(&format!("<b>{}</b>", glib::markup_escape_text(name)))
            }
            None => lbl_device.set_text(""),
        }
        // Note "delai mute" pour les modeles a reveil lent (ex. G533).
        let slow = model
            .as_deref()
            .is_some_and(|n| n.to_ascii_uppercase().contains("G533"));
        lbl_g533.set_text(if slow { s.g533_note } else { "" });
        // Config LED : grisee si le casque connecte n'a pas de RGB (ex. G533).
        let no_rgb = connected && read_has_rgb() == Some(false);
        led_box.set_sensitive(!no_rgb);
        led_note.set_text(if no_rgb { s.led_none } else { s.led_note });
        // Charge affichee seulement casque connecte (sinon la valeur serait perimee).
        let batt = if connected {
            read_battery_status().unwrap_or_else(|| "—".to_string())
        } else {
            "—".to_string()
        };
        lbl_battery.set_text(&format!("{}{}", s.status_battery, batt));
    };
    update();
    glib::timeout_add_seconds_local(1, move || {
        update();
        glib::ControlFlow::Continue
    });

    // Fenetre Adwaita : barre de titre moderne (AdwHeaderBar dans un ToolbarView),
    // et le WM la centre NATIVEMENT au demarrage (plus de hack X11). Le contenu
    // `root` se place sous la barre.
    let toolbar = ToolbarView::new();
    toolbar.add_top_bar(&HeaderBar::new());
    toolbar.set_content(Some(&root));

    let window = ApplicationWindow::builder()
        .application(app)
        .title(s.window_title)
        .default_width(720)
        .default_height(460)
        .resizable(false)
        .content(&toolbar)
        .build();
    {
        let window = window.clone();
        let info = info.clone();
        analyze_btn.connect_clicked(move |_| match run_diagnose() {
            Some(report) => show_diagnostic_dialog(&window, s, &report),
            None => info.set_text(s.analyze_fail),
        });
    }
    {
        let window = window.clone();
        about_btn.connect_clicked(move |_| show_about(&window));
    }

    window.present();
}

fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}
