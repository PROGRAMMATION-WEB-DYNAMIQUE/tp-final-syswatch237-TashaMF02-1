// src/main.rs
// SysWatch — Agent TCP (tourne sur le PC étudiant)
// Corrections appliquées :
//   - collect_snapshot() retourne Result<_, SysWatchError> (Étape 2)
//   - double refresh + sleep pour usage CPU réel
//   - authentification par token (requis par master.rs)
//   - marqueur END après chaque réponse (protocole master.rs)
//   - BufReader pour la lecture ligne par ligne (au lieu de read brut)
//   - verrou Mutex relâché AVANT l'écriture réseau (anti-deadlock)
//   - log() protégé par Mutex global (pas d'entremêlement)
//   - gestion de SysWatchError dans le thread de rafraîchissement
//   - commandes bonus : msg, install, shutdown, reboot, abort

use std::fmt;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use chrono::Local;
use sysinfo::System;

// ── Token d'authentification (doit correspondre à master.rs) ─────────────────
const AUTH_TOKEN: &str = "ENSPD2026";
const LOG_FILE: &str = "syswatch.log";

// ── Structures de données ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct CpuInfo {
    usage_percent: f32,
    core_count: usize,
}

#[derive(Debug, Clone)]
struct MemInfo {
    total_mb: u64,
    used_mb: u64,
    free_mb: u64,
}

#[derive(Debug, Clone)]
struct ProcessInfo {
    pid: u32,
    name: String,
    cpu_percent: f32,
    mem_mb: u64,
}

#[derive(Debug, Clone)]
struct SystemSnapshot {
    cpu: CpuInfo,
    mem: MemInfo,
    processes: Vec<ProcessInfo>,
    timestamp: String,
}

// ── Erreur personnalisée (Étape 2) ────────────────────────────────────────────

#[derive(Debug)]
enum SysWatchError {
    CollectionFailed(String),
}

impl fmt::Display for SysWatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SysWatchError::CollectionFailed(msg) => write!(f, "Erreur collecte : {}", msg),
        }
    }
}

// ── Display (Étape 1) ─────────────────────────────────────────────────────────

impl fmt::Display for CpuInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CPU : {:.1}% | {} cœurs", self.usage_percent, self.core_count)
    }
}

impl fmt::Display for MemInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RAM : {} Mo utilisés / {} Mo total ({} Mo libres)",
            self.used_mb, self.total_mb, self.free_mb
        )
    }
}

impl fmt::Display for ProcessInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PID {:6} | {:20} | CPU {:5.1}% | RAM {:5} Mo",
            self.pid, self.name, self.cpu_percent, self.mem_mb
        )
    }
}

impl fmt::Display for SystemSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "=== Snapshot — {} ===", self.timestamp)?;
        writeln!(f, "{}", self.cpu)?;
        writeln!(f, "{}", self.mem)?;
        writeln!(f, "--- Top 5 processus ---")?;
        for p in &self.processes {
            writeln!(f, "{}", p)?;
        }
        Ok(())
    }
}

// ── Collecte réelle (Étape 2) ─────────────────────────────────────────────────
// CORRECTION : double refresh avec pause pour obtenir un usage CPU non nul

fn collect_snapshot() -> Result<SystemSnapshot, SysWatchError> {
    let mut sys = System::new_all();
    sys.refresh_all();
    thread::sleep(Duration::from_millis(300));
    sys.refresh_all();

    let core_count = sys.cpus().len();
    if core_count == 0 {
        return Err(SysWatchError::CollectionFailed(
            "Aucun CPU détecté".to_string(),
        ));
    }

    let cpu_usage =
        sys.cpus().iter().map(|c| c.cpu_usage()).sum::<f32>() / core_count as f32;

    let total_mb = sys.total_memory() / 1024 / 1024;
    let used_mb = sys.used_memory() / 1024 / 1024;
    let free_mb = sys.free_memory() / 1024 / 1024;

    let mut processes: Vec<ProcessInfo> = sys
        .processes()
        .values()
        .map(|p| ProcessInfo {
            pid: p.pid().as_u32(),
            name: p.name().to_string_lossy().into_owned(),
            cpu_percent: p.cpu_usage(),
            mem_mb: p.memory() / 1024 / 1024,
        })
        .collect();

    // CORRECTION : partial_cmp car f32 n'implémente pas Ord (NaN)
    processes.sort_by(|a, b| {
        b.cpu_percent
            .partial_cmp(&a.cpu_percent)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    processes.truncate(5);

    Ok(SystemSnapshot {
        cpu: CpuInfo { usage_percent: cpu_usage, core_count },
        mem: MemInfo { total_mb, used_mb, free_mb },
        processes,
        timestamp: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    })
}

// ── Barre ASCII (Étape 3) ─────────────────────────────────────────────────────

fn ascii_bar(percent: f32, width: usize) -> String {
    let filled = ((percent / 100.0) * width as f32).round() as usize;
    let filled = filled.min(width);
    format!(
        "[{}{}] {:.1}%",
        "#".repeat(filled),
        "-".repeat(width - filled),
        percent
    )
}

// ── Formatage des réponses réseau (Étape 3) ───────────────────────────────────
// CORRECTION : chaque réponse se termine par "\nEND\n" pour le protocole master

fn format_response(snapshot: &SystemSnapshot, command: &str) -> String {
    let body = match command.trim() {
        "cpu" => {
            format!(
                "=== CPU ===\nCœurs   : {}\nUsage   : {}\nHorloge : {}\n",
                snapshot.cpu.core_count,
                ascii_bar(snapshot.cpu.usage_percent, 30),
                snapshot.timestamp
            )
        }

        "mem" => {
            let used_pct =
                snapshot.mem.used_mb as f32 / snapshot.mem.total_mb.max(1) as f32 * 100.0;
            format!(
                "=== MÉMOIRE ===\nTotal   : {} Mo\nUtilisé : {} Mo\nLibre   : {} Mo\nUsage   : {}\n",
                snapshot.mem.total_mb,
                snapshot.mem.used_mb,
                snapshot.mem.free_mb,
                ascii_bar(used_pct, 30)
            )
        }

        "ps" => {
            let mut out = format!(
                "=== TOP 5 PROCESSUS ===\n{:<8} {:<22} {:>8} {:>10}\n{}\n",
                "PID", "NOM", "CPU%", "RAM(Mo)", "-".repeat(52)
            );
            for p in &snapshot.processes {
                out.push_str(&format!(
                    "{:<8} {:<22} {:>7.1}% {:>9} Mo\n",
                    p.pid, p.name, p.cpu_percent, p.mem_mb
                ));
            }
            out
        }

        "all" => format!(
            "{}\n{}\n{}",
            format_response(snapshot, "cpu"),
            format_response(snapshot, "mem"),
            format_response(snapshot, "ps")
        ),

        "help" => {
            "Commandes disponibles :\n\
             cpu      — usage CPU\n\
             mem      — état de la RAM\n\
             ps       — top 5 processus\n\
             all      — toutes les infos\n\
             msg <x>  — afficher un message\n\
             shutdown — éteindre la machine\n\
             reboot   — redémarrer\n\
             abort    — annuler extinction\n\
             help     — cette aide\n\
             quit     — fermer la connexion\n"
                .to_string()
        }

        "quit" => "Au revoir.\n".to_string(),

        other => {
            // Commandes bonus interprétées par l'agent
            if let Some(msg) = other.strip_prefix("msg ") {
                format!("[MSG] {}\n", msg)
            } else if let Some(pkg) = other.strip_prefix("install ") {
                format!("[INSTALL] Simulation installation de '{}'\n", pkg)
            } else if other == "shutdown" {
                #[cfg(target_os = "linux")]
                std::process::Command::new("shutdown")
                    .args(["-h", "+1"])
                    .spawn()
                    .ok();
                "Extinction programmée dans 1 minute.\n".to_string()
            } else if other == "reboot" {
                #[cfg(target_os = "linux")]
                std::process::Command::new("reboot").spawn().ok();
                "Redémarrage en cours...\n".to_string()
            } else if other == "abort" {
                #[cfg(target_os = "linux")]
                std::process::Command::new("shutdown")
                    .arg("-c")
                    .spawn()
                    .ok();
                "Extinction annulée.\n".to_string()
            } else {
                format!("Commande inconnue : '{}'. Tapez 'help'.\n", other)
            }
        }
    };

    // Le protocole master.rs attend "END" comme marqueur de fin de réponse
    // Éviter la duplication quand format_response s'appelle récursivement ("all")
    if command.trim() == "all" {
        body
    } else {
        body
    }
}

// ── Journalisation thread-safe (Étape 5) ──────────────────────────────────────
// CORRECTION : Mutex global pour éviter l'entremêlement des lignes de log

static LOG_MUTEX: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();

fn log(msg: &str) {
    let mutex = LOG_MUTEX.get_or_init(|| Mutex::new(()));
    let _guard = mutex.lock().unwrap_or_else(|e| e.into_inner());

    let line = format!("[{}] {}\n", Local::now().format("%Y-%m-%d %H:%M:%S"), msg);

    // CORRECTION : if let Ok() au lieu de .unwrap() — un échec de log ne doit
    // pas faire planter le serveur
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(LOG_FILE) {
        let _ = file.write_all(line.as_bytes());
    }
}

// ── Gestion d'un client (Étape 4) ────────────────────────────────────────────

fn handle_client(stream: TcpStream, snapshot: Arc<Mutex<SystemSnapshot>>) {
    let peer = stream
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "inconnu".to_string());

    log(&format!("CONNEXION {}", peer));

    // CORRECTION : try_clone() pour séparer reader et writer sur le même socket
    let mut writer = match stream.try_clone() {
        Ok(w) => w,
        Err(e) => {
            log(&format!("ERREUR clone stream {} : {}", peer, e));
            return;
        }
    };

    // CORRECTION : BufReader pour la lecture ligne par ligne (robuste aux
    // paquets TCP fragmentés), au lieu du read brut de 512 octets
    let reader = BufReader::new(stream);

    // ── Authentification (protocole master.rs) ────────────────────────────────
    let _ = write!(writer, "TOKEN: ");
    let _ = writer.flush();

    let mut lines = reader.lines();

    let token = match lines.next() {
        Some(Ok(t)) => t,
        _ => {
            let _ = writeln!(writer, "ERREUR lecture token");
            log(&format!("AUTH ECHEC (lecture) {}", peer));
            return;
        }
    };

    if token.trim() != AUTH_TOKEN {
        let _ = writeln!(writer, "ERREUR token invalide");
        log(&format!("AUTH REFUS {} (token: '{}')", peer, token.trim()));
        return;
    }

    let _ = writeln!(writer, "OK");
    log(&format!("AUTH OK {}", peer));

    // ── Boucle de commandes ───────────────────────────────────────────────────
    for line in lines {
        let command = match line {
            Ok(l) => l.trim().to_lowercase(),
            Err(_) => break,
        };

        if command.is_empty() {
            continue;
        }

        log(&format!("CMD {} > {}", peer, command));

        // CORRECTION : le verrou est relâché AVANT l'écriture réseau
        // (évite de tenir le Mutex pendant une I/O potentiellement longue)
        let response = {
            let snap = snapshot.lock().unwrap_or_else(|e| e.into_inner());
            format_response(&snap, &command)
        }; // ← MutexGuard droppé ici

        // Envoi de la réponse + marqueur END (protocole master.rs)
        if write!(writer, "{}\nEND\n", response).is_err() {
            break;
        }
        if writer.flush().is_err() {
            break;
        }

        if command == "quit" {
            break;
        }
    }

    log(&format!("DECONNEXION {}", peer));
}

// ── Thread de rafraîchissement (Étape 4) ─────────────────────────────────────

fn refresh_loop(snapshot: Arc<Mutex<SystemSnapshot>>) {
    loop {
        thread::sleep(Duration::from_secs(5));
        // CORRECTION : gérer l'erreur de collect_snapshot au lieu de paniquer
        match collect_snapshot() {
            Ok(new_snap) => {
                let mut snap = snapshot.lock().unwrap_or_else(|e| e.into_inner());
                *snap = new_snap;
                log("REFRESH snapshot OK");
            }
            Err(e) => {
                log(&format!("REFRESH ERREUR : {}", e));
            }
        }
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    println!("=== SysWatch Agent démarrage ===");

    // Collecte initiale
    let initial = match collect_snapshot() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Erreur collecte initiale : {}", e);
            std::process::exit(1);
        }
    };

    println!("{}", initial);
    log("DEMARRAGE agent SysWatch");

    let shared = Arc::new(Mutex::new(initial));

    // Thread de rafraîchissement toutes les 5 s
    {
        let clone = Arc::clone(&shared);
        thread::spawn(move || refresh_loop(clone));
    }

    // Serveur TCP
    let listener = TcpListener::bind("0.0.0.0:7878").unwrap_or_else(|e| {
        eprintln!("Impossible de démarrer le serveur : {}", e);
        std::process::exit(1);
    });

    println!("Serveur en écoute sur 0.0.0.0:7878");
    println!("Token d'authentification : {}", AUTH_TOKEN);

    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let clone = Arc::clone(&shared);
                thread::spawn(move || handle_client(stream, clone));
            }
            Err(e) => eprintln!("Erreur connexion entrante : {}", e),
        }
    }
}
