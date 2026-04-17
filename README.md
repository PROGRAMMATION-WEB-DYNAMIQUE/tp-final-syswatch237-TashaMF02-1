# SysWatch — Moniteur Système en Réseau

**TP Intégral Rust | Génie Logiciel L4 | ENSPD 2025-2026**

## Description

SysWatch est un serveur TCP interactif qui collecte les métriques réelles d'une machine (CPU, RAM, processus) et répond aux commandes de n'importe quel client connecté.

Le projet contient deux binaires :
- **`syswatch`** — l'agent qui tourne sur chaque PC étudiant
- **`syswatch-master`** — l'interface maître du professeur pour piloter toutes les machines

---

## Mise en place

```bash
# Cloner le dépôt
git clone <url-du-repo>
cd syswatch

# Compiler les deux binaires
cargo build

# Lancer l'agent (sur le PC étudiant)
cargo run --bin syswatch

# Lancer le master (sur le PC du professeur)
cargo run --bin syswatch-master
```

---

## Dépendances

```toml
sysinfo = "0.30"   # Collecte des métriques système
chrono  = "0.4"    # Horodatage des logs
```

---

## Commandes disponibles (agent)

| Commande | Description |
|----------|-------------|
| `cpu` | Affiche l'usage CPU avec barre ASCII |
| `mem` | Affiche l'état de la RAM avec barre ASCII |
| `ps` | Affiche le top 5 des processus (par CPU) |
| `all` | Affiche toutes les métriques |
| `msg <texte>` | Affiche un message sur la machine distante |
| `shutdown` | Programme l'extinction |
| `reboot` | Redémarre la machine |
| `abort` | Annule l'extinction programmée |
| `help` | Liste les commandes |
| `quit` | Ferme la connexion |

---

## Architecture

```
┌─────────────────────┐        TCP :7878        ┌──────────────────────┐
│   syswatch-master   │◄───────────────────────►│   syswatch (agent)   │
│  (PC professeur)    │   token: ENSPD2026      │   (PC étudiant)      │
└─────────────────────┘                         └──────────────────────┘
                                                         │
                                              ┌──────────┴──────────┐
                                              │   syswatch.log      │
                                              │  (journalisation)   │
                                              └─────────────────────┘
```

### Concepts Rust illustrés

| Étape | Concepts |
|-------|----------|
| 1 | `struct`, `impl fmt::Display`, `Vec<T>`, `derive(Debug, Clone)` |
| 2 | `Result<T,E>`, enum d'erreur, `.sort_by()`, `.partial_cmp()` |
| 3 | Pattern matching sur `&str`, barres ASCII, formatage aligné |
| 4 | `TcpListener`, `thread::spawn`, `Arc<Mutex<T>>`, `BufReader` |
| 5 | `OpenOptions`, mode append, log thread-safe via `OnceLock<Mutex>` |

---

## Protocole de communication

1. Le client se connecte sur le port 7878
2. L'agent envoie `TOKEN: `
3. Le client répond avec le token (`ENSPD2026`)
4. L'agent répond `OK` si le token est valide
5. Chaque réponse de commande se termine par `\nEND\n` (lu par le master)

---

## Auteur

**TashaMF02** — Génie Logiciel L4, ENSPD 2025-2026
