// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Escritura atómica de ficheros: fichero temporal en el MISMO directorio,
//! `flush` + `sync_all` y `rename` sobre el destino. Un cierre inesperado o
//! un corte de luz deja el fichero anterior íntegro o el nuevo completo,
//! nunca uno truncado. Si algo falla, el temporal se borra.
//!
//! Lo usan `export_markdown_to_file` (aquí) y las escrituras de estado de
//! `pdf_android::persist`.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Escribe `contents` en `path` de forma atómica (ver doc del módulo).
pub fn write_atomic(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    write_atomic_with(path, |file| file.write_all(contents.as_ref()))
}

/// Igual que [`write_atomic`], pero el contenido lo escribe `fill` sobre el
/// fichero temporal. Si `fill` falla, el destino no se toca.
pub fn write_atomic_with<F>(path: &Path, fill: F) -> io::Result<()>
where
    F: FnOnce(&mut File) -> io::Result<()>,
{
    let temp = temp_path_for(path)?;
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        fill(&mut file)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    // Persistir la entrada de directorio del rename (best-effort: no todos
    // los sistemas de ficheros permiten sincronizar un directorio).
    if let Ok(dir) = File::open(parent_dir(path)) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Directorio padre de `path` (`.` para rutas relativas sin directorio).
fn parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    }
}

/// Temporal oculto y único junto al destino (mismo sistema de ficheros, así
/// el `rename` es atómico): `.<nombre>.<pid>-<seq>.tmp`.
fn temp_path_for(path: &Path) -> io::Result<PathBuf> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let Some(name) = path.file_name() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "la ruta de destino no tiene nombre de fichero",
        ));
    };
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let mut temp_name = std::ffi::OsString::from(".");
    temp_name.push(name);
    temp_name.push(format!(".{}-{seq}.tmp", std::process::id()));
    Ok(parent_dir(path).join(temp_name))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use std::fs;
    use std::sync::atomic::AtomicU32;

    /// Directorio temporal único por test (se borra al soltarlo).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            static SEQ: AtomicU32 = AtomicU32::new(0);
            let seq = SEQ.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "pdflector-fsutil-{name}-{}-{seq}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Nombres de las entradas de `dir`, ordenados.
    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn writes_new_file_and_replaces_existing() {
        let tmp = TempDir::new("writes");
        let path = tmp.0.join("state.json");
        write_atomic(&path, "uno").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "uno");
        write_atomic(&path, b"dos, mas largo").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "dos, mas largo");
        assert_eq!(entries(&tmp.0), vec!["state.json"], "sin temporales");
    }

    #[test]
    fn prior_file_intact_and_no_temp_when_fill_fails() {
        let tmp = TempDir::new("fill-fails");
        let path = tmp.0.join("library.json");
        fs::write(&path, "anterior").unwrap();

        let err = write_atomic_with(&path, |f| {
            f.write_all(b"a medias")?;
            Err(io::Error::other("fallo simulado"))
        })
        .unwrap_err();

        assert_eq!(err.to_string(), "fallo simulado");
        assert_eq!(fs::read_to_string(&path).unwrap(), "anterior");
        assert_eq!(entries(&tmp.0), vec!["library.json"], "sin temporales");
    }

    #[test]
    fn target_is_directory_fails_without_leftovers() {
        let tmp = TempDir::new("is-dir");
        let path = tmp.0.join("recents.json");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("dentro"), "x").unwrap();

        assert!(write_atomic(&path, "nuevo").is_err());

        assert!(path.is_dir(), "el destino previo sigue intacto");
        assert_eq!(fs::read_to_string(path.join("dentro")).unwrap(), "x");
        assert_eq!(entries(&tmp.0), vec!["recents.json"], "sin temporales");
    }

    #[test]
    fn missing_parent_fails() {
        let tmp = TempDir::new("no-parent");
        let path = tmp.0.join("no-existe").join("x.json");
        assert!(write_atomic(&path, "x").is_err());
        assert!(entries(&tmp.0).is_empty());
    }

    /// El reemplazo es por `rename` (inodo nuevo), no truncando el fichero en
    /// sitio: un hard link al fichero viejo conserva el contenido anterior.
    #[cfg(unix)]
    #[test]
    fn replaces_by_rename_not_in_place() {
        let tmp = TempDir::new("rename");
        let path = tmp.0.join("state.json");
        let old = tmp.0.join("old-link");
        fs::write(&path, "anterior").unwrap();
        fs::hard_link(&path, &old).unwrap();

        write_atomic(&path, "nuevo").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "nuevo");
        assert_eq!(fs::read_to_string(&old).unwrap(), "anterior");
        assert_eq!(entries(&tmp.0), vec!["old-link", "state.json"]);
    }

    #[test]
    fn colliding_temp_file_is_not_deleted_on_already_exists() {
        let tmp = TempDir::new("collide");
        let path = tmp.0.join("state.json");
        let probe = temp_path_for(&path).unwrap();
        let probe_name = probe.file_name().unwrap().to_str().unwrap();
        let prefix = format!(".state.json.{}-", std::process::id());
        let seq: u64 = probe_name
            .strip_prefix(&prefix)
            .and_then(|s| s.strip_suffix(".tmp"))
            .unwrap()
            .parse()
            .unwrap();

        let precreated: Vec<PathBuf> = (1..=32)
            .map(|offset| {
                let p = tmp.0.join(format!("{prefix}{}.tmp", seq + offset));
                fs::write(&p, "keep-me").unwrap();
                p
            })
            .collect();

        let err = write_atomic(&path, "new").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        for p in &precreated {
            assert!(p.exists(), "colliding temp file was deleted: {p:?}");
            assert_eq!(fs::read_to_string(p).unwrap(), "keep-me");
        }
    }
}
