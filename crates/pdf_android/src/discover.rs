// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Worker de fondo y caché de feeds para la funcionalidad Discover de arXiv.
//!
//! Conforme al diseño de `docs/superpowers/specs/2026-09-10-arxiv-fast-access-design.md`:
//! - 1 hilo, 1 conexión, rate limit respetado vía `ArxivClient`.
//! - Descarga streaming en bloques de 8KB a fichero temporal `.part` y rename atómico.
//! - FeedCache en disco: límite ≤ 4 MiB, LRU, TTL de 10 minutos.
//! - El hilo UI nunca bloquea; comunicación por canales `std::sync::mpsc`.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::{JoinHandle, spawn};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pdf_core::arxiv::{ArxivClient, ArxivEntry, ArxivError, ArxivQuery, parse_arxiv_id};

/// Límite máximo de la caché en disco de feeds (4 MiB).
pub const FEED_CACHE_MAX_BYTES: u64 = 4 * 1024 * 1024;
/// Tiempo de vida máximo de una entrada de caché de feeds (10 minutos).
pub const FEED_CACHE_TTL_SECS: u64 = 600;

/// Genera una clave determinista para la caché del feed según las categorías y el offset de inicio.
pub fn feed_cache_key(cats: &[String], start: usize) -> String {
    let mut sorted_cats = cats.to_vec();
    sorted_cats.sort();
    let joined = sorted_cats.join(",");
    format!("feed_{}_start_{}", joined, start)
}

/// Clave de caché para búsquedas por texto libre.
pub fn search_cache_key(query: &str, start: usize) -> String {
    let sanitized: String = query
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    format!("search_{}_start_{}", sanitized, start)
}

/// Comandos enviados desde la UI al worker Discover.
#[derive(Debug, Clone)]
pub enum DiscoverCmd {
    /// Carga inicial o recarga del feed para las categorías indicadas.
    Feed { cats: Vec<String> },
    /// Búsqueda por texto libre o expresión de consulta.
    Search(String),
    /// Cargar página siguiente (paginación) para el feed o la búsqueda activa.
    More,
    /// Descargar el paper con el identificador arXiv especificado y metadatos opcionales.
    Download {
        id: String,
        entry: Option<ArxivEntry>,
    },
    /// Cancelar la operación o descarga en vuelo.
    Cancel,
    /// Detener el worker.
    Stop,
}

/// Mensajes devueltos por el worker Discover al hilo de UI.
#[derive(Debug, Clone)]
pub enum DiscoverMsg {
    /// Resultado de carga del feed (reemplaza o inicializa lista).
    FeedLoaded {
        entries: Vec<ArxivEntry>,
        has_more: bool,
    },
    /// Resultado de búsqueda.
    SearchLoaded {
        query: String,
        entries: Vec<ArxivEntry>,
        has_more: bool,
    },
    /// Nuevas entradas añadidas por paginación (`More`).
    MoreLoaded {
        entries: Vec<ArxivEntry>,
        has_more: bool,
    },
    /// Fallo en consulta de feed o búsqueda.
    QueryFailed { error: String },
    /// Progreso de descarga de un paper (bytes acumulados).
    DownloadProgress { id: String, bytes: u64 },
    /// Descarga finalizada con éxito y movida a su ruta destino.
    DownloadFinished {
        id: String,
        path: String,
        entry: Option<ArxivEntry>,
    },
    /// Error en la descarga del paper.
    DownloadFailed { id: String, error: String },
}

/// Estructura de metadatos de una entrada en disco del FeedCache.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
struct CacheRecord {
    timestamp: u64,
    size_bytes: u64,
    entries: Vec<ArxivEntry>,
}

/// Caché en disco LRU de respuestas de feeds y búsquedas (≤ 4 MiB, TTL 10 min).
pub struct FeedCache {
    dir: PathBuf,
    max_bytes: u64,
    ttl: Duration,
}

impl FeedCache {
    pub fn new(cache_dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&cache_dir);
        Self {
            dir: cache_dir,
            max_bytes: FEED_CACHE_MAX_BYTES,
            ttl: Duration::from_secs(FEED_CACHE_TTL_SECS),
        }
    }

    fn file_path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{}.json", key))
    }

    pub fn get(&self, key: &str) -> Option<Vec<ArxivEntry>> {
        let p = self.file_path(key);
        let data = fs::read_to_string(&p).ok()?;
        let record: CacheRecord = serde_json::from_str(&data).ok()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if now.saturating_sub(record.timestamp) > self.ttl.as_secs() {
            let _ = fs::remove_file(&p);
            return None;
        }
        let _ = std::fs::File::options()
            .write(true)
            .open(&p)
            .and_then(|f| f.set_modified(SystemTime::now()));
        Some(record.entries)
    }

    pub fn put(&self, key: &str, entries: &[ArxivEntry]) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let p = self.file_path(key);
        let record = CacheRecord {
            timestamp: now,
            size_bytes: 0,
            entries: entries.to_vec(),
        };
        if let Ok(json) = serde_json::to_string(&record) {
            let size = json.len() as u64;
            let record = CacheRecord {
                timestamp: now,
                size_bytes: size,
                entries: entries.to_vec(),
            };
            if let Ok(json) = serde_json::to_string(&record) {
                let _ = fs::write(&p, json);
            }
        }
        self.prune();
    }

    /// Limpia entradas expiradas o expulsa LRU si el tamaño total excede `max_bytes`.
    pub fn prune(&self) {
        let Ok(rd) = fs::read_dir(&self.dir) else {
            return;
        };
        let mut items = Vec::new();
        let mut total_bytes: u64 = 0;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                if let Ok(meta) = entry.metadata() {
                    let len = meta.len();
                    let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    let mtime_secs = mtime
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    if now.saturating_sub(mtime_secs) > self.ttl.as_secs() {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    total_bytes += len;
                    items.push((path, len, mtime_secs));
                }
            }
        }

        if total_bytes > self.max_bytes {
            // Ordenar por mtime ascendente (los más antiguos primero)
            items.sort_by_key(|item| item.2);
            for (path, len, _) in items {
                if total_bytes <= self.max_bytes {
                    break;
                }
                if fs::remove_file(&path).is_ok() {
                    total_bytes = total_bytes.saturating_sub(len);
                }
            }
        }
    }
}

/// Estado interno activo en el hilo de fondo del worker.
enum ActiveMode {
    None,
    Feed { cats: Vec<String>, start: usize },
    Search { query: String, start: usize },
}

/// Worker actor de Discover ejecutado en un hilo dedicado con 1 sola conexión HTTP.
pub struct DiscoverWorker {
    tx: Sender<DiscoverCmd>,
    cancel_flag: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl DiscoverWorker {
    pub fn spawn(cache_dir: PathBuf, internal_dir: PathBuf) -> (Self, Receiver<DiscoverMsg>) {
        let (cmd_tx, cmd_rx) = channel::<DiscoverCmd>();
        let (msg_tx, msg_rx) = channel::<DiscoverMsg>();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let cancel_flag_worker = Arc::clone(&cancel_flag);

        let handle = spawn(move || {
            let client = match ArxivClient::new() {
                Ok(c) => c,
                Err(e) => {
                    log::error!("DiscoverWorker: ArxivClient::new failed: {e}");
                    let _ = msg_tx.send(DiscoverMsg::QueryFailed {
                        error: format!("No se pudo inicializar cliente arXiv: {e}"),
                    });
                    return;
                }
            };

            let feed_cache = FeedCache::new(cache_dir);
            let mut active_mode = ActiveMode::None;
            let page_size = 25;

            while let Ok(cmd) = cmd_rx.recv() {
                // Si había preemption o comandos más recientes, procesarlos
                let cmd = match drain_newer_cmd(&cmd_rx, cmd) {
                    Some(c) => c,
                    None => continue,
                };

                match cmd {
                    DiscoverCmd::Stop => break,
                    DiscoverCmd::Cancel => {
                        cancel_flag_worker.store(true, Ordering::SeqCst);
                        continue;
                    }
                    DiscoverCmd::Feed { cats } => {
                        cancel_flag_worker.store(false, Ordering::SeqCst);
                        let cache_key = feed_cache_key(&cats, 0);
                        if let Some(cached) = feed_cache.get(&cache_key) {
                            let has_more = cached.len() >= page_size;
                            active_mode = ActiveMode::Feed {
                                cats: cats.clone(),
                                start: cached.len(),
                            };
                            let _ = msg_tx.send(DiscoverMsg::FeedLoaded {
                                entries: cached,
                                has_more,
                            });
                            continue;
                        }

                        // Construir query arXiv
                        let query_str = if cats.is_empty() {
                            "cat:cs.AI OR cat:cs.LG".to_string()
                        } else {
                            cats.iter()
                                .map(|c| format!("cat:{c}"))
                                .collect::<Vec<_>>()
                                .join(" OR ")
                        };

                        let query = ArxivQuery::new()
                            .with_search_query(query_str)
                            .with_start(0)
                            .with_max_results(page_size)
                            .with_sort_by("submittedDate")
                            .with_sort_order("descending");

                        match client.search(&query) {
                            Ok(entries) => {
                                if cancel_flag_worker.load(Ordering::SeqCst) {
                                    continue;
                                }
                                feed_cache.put(&cache_key, &entries);
                                let has_more = entries.len() >= page_size;
                                active_mode = ActiveMode::Feed {
                                    cats,
                                    start: entries.len(),
                                };
                                let _ = msg_tx.send(DiscoverMsg::FeedLoaded { entries, has_more });
                            }
                            Err(e) => {
                                if !cancel_flag_worker.load(Ordering::SeqCst) {
                                    let _ = msg_tx.send(DiscoverMsg::QueryFailed {
                                        error: format!("{e}"),
                                    });
                                }
                            }
                        }
                    }
                    DiscoverCmd::Search(q) => {
                        cancel_flag_worker.store(false, Ordering::SeqCst);
                        let trimmed = q.trim().to_string();
                        if trimmed.is_empty() {
                            continue;
                        }
                        let cache_key = search_cache_key(&trimmed, 0);
                        if let Some(cached) = feed_cache.get(&cache_key) {
                            let has_more = cached.len() >= page_size;
                            active_mode = ActiveMode::Search {
                                query: trimmed.clone(),
                                start: cached.len(),
                            };
                            let _ = msg_tx.send(DiscoverMsg::SearchLoaded {
                                query: trimmed,
                                entries: cached,
                                has_more,
                            });
                            continue;
                        }

                        let is_formatted = trimmed.contains(':');
                        let query_param = if is_formatted {
                            trimmed.clone()
                        } else {
                            format!("all:{}", trimmed)
                        };

                        let query = ArxivQuery::new()
                            .with_search_query(query_param)
                            .with_start(0)
                            .with_max_results(page_size)
                            .with_sort_by("relevance")
                            .with_sort_order("descending");

                        match client.search(&query) {
                            Ok(entries) => {
                                if cancel_flag_worker.load(Ordering::SeqCst) {
                                    continue;
                                }
                                feed_cache.put(&cache_key, &entries);
                                let has_more = entries.len() >= page_size;
                                active_mode = ActiveMode::Search {
                                    query: trimmed.clone(),
                                    start: entries.len(),
                                };
                                let _ = msg_tx.send(DiscoverMsg::SearchLoaded {
                                    query: trimmed,
                                    entries,
                                    has_more,
                                });
                            }
                            Err(e) => {
                                if !cancel_flag_worker.load(Ordering::SeqCst) {
                                    let _ = msg_tx.send(DiscoverMsg::QueryFailed {
                                        error: format!("{e}"),
                                    });
                                }
                            }
                        }
                    }
                    DiscoverCmd::More => {
                        cancel_flag_worker.store(false, Ordering::SeqCst);
                        match &mut active_mode {
                            ActiveMode::None => {}
                            ActiveMode::Feed { cats, start } => {
                                let cur_start = *start;
                                let cache_key = feed_cache_key(cats, cur_start);
                                if let Some(cached) = feed_cache.get(&cache_key) {
                                    let has_more = cached.len() >= page_size;
                                    *start += cached.len();
                                    let _ = msg_tx.send(DiscoverMsg::MoreLoaded {
                                        entries: cached,
                                        has_more,
                                    });
                                    continue;
                                }

                                let query_str = if cats.is_empty() {
                                    "cat:cs.AI OR cat:cs.LG".to_string()
                                } else {
                                    cats.iter()
                                        .map(|c| format!("cat:{c}"))
                                        .collect::<Vec<_>>()
                                        .join(" OR ")
                                };
                                let query = ArxivQuery::new()
                                    .with_search_query(query_str)
                                    .with_start(cur_start)
                                    .with_max_results(page_size)
                                    .with_sort_by("submittedDate")
                                    .with_sort_order("descending");

                                match client.search(&query) {
                                    Ok(entries) => {
                                        if cancel_flag_worker.load(Ordering::SeqCst) {
                                            continue;
                                        }
                                        feed_cache.put(&cache_key, &entries);
                                        let has_more = entries.len() >= page_size;
                                        *start += entries.len();
                                        let _ = msg_tx
                                            .send(DiscoverMsg::MoreLoaded { entries, has_more });
                                    }
                                    Err(e) => {
                                        if !cancel_flag_worker.load(Ordering::SeqCst) {
                                            let _ = msg_tx.send(DiscoverMsg::QueryFailed {
                                                error: format!("{e}"),
                                            });
                                        }
                                    }
                                }
                            }
                            ActiveMode::Search { query, start } => {
                                let cur_start = *start;
                                let cache_key = search_cache_key(query, cur_start);
                                if let Some(cached) = feed_cache.get(&cache_key) {
                                    let has_more = cached.len() >= page_size;
                                    *start += cached.len();
                                    let _ = msg_tx.send(DiscoverMsg::MoreLoaded {
                                        entries: cached,
                                        has_more,
                                    });
                                    continue;
                                }

                                let is_formatted = query.contains(':');
                                let query_param = if is_formatted {
                                    query.clone()
                                } else {
                                    format!("all:{}", query)
                                };

                                let q = ArxivQuery::new()
                                    .with_search_query(query_param)
                                    .with_start(cur_start)
                                    .with_max_results(page_size)
                                    .with_sort_by("relevance")
                                    .with_sort_order("descending");

                                match client.search(&q) {
                                    Ok(entries) => {
                                        if cancel_flag_worker.load(Ordering::SeqCst) {
                                            continue;
                                        }
                                        feed_cache.put(&cache_key, &entries);
                                        let has_more = entries.len() >= page_size;
                                        *start += entries.len();
                                        let _ = msg_tx
                                            .send(DiscoverMsg::MoreLoaded { entries, has_more });
                                    }
                                    Err(e) => {
                                        if !cancel_flag_worker.load(Ordering::SeqCst) {
                                            let _ = msg_tx.send(DiscoverMsg::QueryFailed {
                                                error: format!("{e}"),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                    DiscoverCmd::Download { id, entry } => {
                        cancel_flag_worker.store(false, Ordering::SeqCst);
                        let (canonical_id, _) = match parse_arxiv_id(&id) {
                            Ok(pair) => pair,
                            Err(e) => {
                                let _ = msg_tx.send(DiscoverMsg::DownloadFailed {
                                    id,
                                    error: format!("ID inválido: {e}"),
                                });
                                continue;
                            }
                        };

                        // Directorio de destino: internal/pdfs/
                        let pdfs_dir = internal_dir.join("pdfs");
                        if let Err(e) = fs::create_dir_all(&pdfs_dir) {
                            let _ = msg_tx.send(DiscoverMsg::DownloadFailed {
                                id: canonical_id,
                                error: format!("Error creando directorio pdfs: {e}"),
                            });
                            continue;
                        }

                        // Convierte / a _ solo para el nombre de fichero en disco
                        let safe_filename = format!("arxiv_{}.pdf", canonical_id.replace('/', "_"));
                        let part_filename = format!("{}.part", safe_filename);
                        let final_path = pdfs_dir.join(&safe_filename);
                        let part_path = pdfs_dir.join(&part_filename);

                        // Streaming writer con progreso en bloques de 8KB y chequeo de cancelación
                        struct ProgressWriter<'a> {
                            file: File,
                            id: String,
                            bytes_written: u64,
                            last_reported: Instant,
                            msg_tx: &'a Sender<DiscoverMsg>,
                            cancel: &'a AtomicBool,
                        }

                        impl<'a> Write for ProgressWriter<'a> {
                            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                                if self.cancel.load(Ordering::SeqCst) {
                                    return Err(io::Error::new(
                                        io::ErrorKind::Interrupted,
                                        "Download cancelled",
                                    ));
                                }
                                let n = self.file.write(buf)?;
                                self.bytes_written += n as u64;
                                if self.last_reported.elapsed() >= Duration::from_millis(200) {
                                    let _ = self.msg_tx.send(DiscoverMsg::DownloadProgress {
                                        id: self.id.clone(),
                                        bytes: self.bytes_written,
                                    });
                                    self.last_reported = Instant::now();
                                }
                                Ok(n)
                            }

                            fn flush(&mut self) -> io::Result<()> {
                                self.file.flush()
                            }
                        }

                        let part_file = match File::create(&part_path) {
                            Ok(f) => f,
                            Err(e) => {
                                let _ = msg_tx.send(DiscoverMsg::DownloadFailed {
                                    id: canonical_id,
                                    error: format!("Error creando fichero .part: {e}"),
                                });
                                continue;
                            }
                        };

                        let mut writer = ProgressWriter {
                            file: part_file,
                            id: canonical_id.clone(),
                            bytes_written: 0,
                            last_reported: Instant::now(),
                            msg_tx: &msg_tx,
                            cancel: &cancel_flag_worker,
                        };

                        let download_res = client.download_to(&canonical_id, &mut writer);
                        drop(writer);

                        if cancel_flag_worker.load(Ordering::SeqCst) {
                            let _ = fs::remove_file(&part_path);
                            let _ = msg_tx.send(DiscoverMsg::DownloadFailed {
                                id: canonical_id,
                                error: "Descarga cancelada".to_string(),
                            });
                            continue;
                        }

                        match download_res {
                            Ok(_outcome) => {
                                // Rename atómico de .part al nombre final
                                if let Err(e) = fs::rename(&part_path, &final_path) {
                                    let _ = fs::remove_file(&part_path);
                                    let _ = msg_tx.send(DiscoverMsg::DownloadFailed {
                                        id: canonical_id,
                                        error: format!("Error renombrando .part a final: {e}"),
                                    });
                                } else {
                                    let path_str = final_path.display().to_string();
                                    let _ = msg_tx.send(DiscoverMsg::DownloadFinished {
                                        id: canonical_id,
                                        path: path_str,
                                        entry,
                                    });
                                }
                            }
                            Err(e) => {
                                let _ = fs::remove_file(&part_path);
                                let _ = msg_tx.send(DiscoverMsg::DownloadFailed {
                                    id: canonical_id,
                                    error: format!("{e}"),
                                });
                            }
                        }
                    }
                }
            }
        });

        (
            Self {
                tx: cmd_tx,
                cancel_flag,
                handle: Some(handle),
            },
            msg_rx,
        )
    }

    pub fn send(&self, cmd: DiscoverCmd) {
        let _ = self.tx.send(cmd);
    }

    pub fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
        let _ = self.tx.send(DiscoverCmd::Cancel);
    }
}

impl Drop for DiscoverWorker {
    fn drop(&mut self) {
        let _ = self.tx.send(DiscoverCmd::Stop);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Drena comandos más nuevos si hay ráfagas de input (preemption).
fn drain_newer_cmd(rx: &Receiver<DiscoverCmd>, initial: DiscoverCmd) -> Option<DiscoverCmd> {
    let mut cur = initial;
    while let Ok(newer) = rx.try_recv() {
        match newer {
            DiscoverCmd::Stop => return Some(DiscoverCmd::Stop),
            DiscoverCmd::Cancel => return Some(DiscoverCmd::Cancel),
            other => cur = other,
        }
    }
    Some(cur)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_cache_key_stable() {
        assert_eq!(
            feed_cache_key(&["cs.AI".into()], 0),
            feed_cache_key(&["cs.AI".into()], 0)
        );
        assert_eq!(
            feed_cache_key(&["cs.LG".into(), "cs.AI".into()], 25),
            feed_cache_key(&["cs.AI".into(), "cs.LG".into()], 25)
        );
    }

    #[test]
    fn search_cache_key_stable() {
        assert_eq!(
            search_cache_key("attention is all you need", 0),
            search_cache_key("attention is all you need", 0)
        );
    }
}
