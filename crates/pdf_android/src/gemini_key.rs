// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Clave de Gemini introducida DENTRO de la app (PR-02).
//!
//! Sustituye a la clave embebida en el APK en tiempo de compilación
//! (`include_str!("../google_key.txt")`, eliminado): el usuario la pega en
//! Ajustes de lectura → Estudio → Clave de Gemini y se guarda en el
//! almacenamiento interno PRIVADO de la app (`internal_data_path()`,
//! `/data/user/0/<paquete>/files/`, solo legible por el UID de la app).
//!
//! Protección:
//! - Fichero `gemini_api_key` con escritura atómica (`pdf_core::write_atomic`)
//!   y permisos 0600.
//! - Nunca se loguea: `GeminiKey` implementa `Debug` enmascarado y la UI solo
//!   muestra los 4 últimos caracteres (`mask`).

use std::io;
use std::path::{Path, PathBuf};

use pdf_core::write_atomic;

/// Longitud mínima aceptada (las claves de AI Studio tienen 39 caracteres;
/// mismo umbral que el antiguo aviso de "falta la clave").
pub(crate) const MIN_KEY_LEN: usize = 20;

/// Aviso del panel de IA cuando no hay clave guardada.
pub(crate) const MISSING_KEY_MSG: &str = "Falta tu clave de Gemini: añádela en Ajustes de lectura → Estudio → Clave de Gemini (se obtiene gratis en aistudio.google.com/apikey).";

/// Clave de Gemini validada. `Debug` NO revela la clave.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct GeminiKey(String);

/// Motivo por el que el texto introducido no es una clave válida.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyError {
    Empty,
    TooShort,
    InvalidChars,
}

impl KeyError {
    /// Mensaje para el usuario (panel de la clave).
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Empty => "Pega la clave antes de guardar.",
            Self::TooShort => "La clave es demasiado corta: cópiala completa de AI Studio.",
            Self::InvalidChars => {
                "La clave solo puede tener letras, números, guiones y guiones bajos."
            }
        }
    }
}

impl GeminiKey {
    /// Valida el texto pegado: se descartan los espacios y saltos de línea
    /// (pegados por accidente) y se exige `[A-Za-z0-9_-]` y `MIN_KEY_LEN`.
    pub(crate) fn parse(input: &str) -> Result<Self, KeyError> {
        let key: String = input.chars().filter(|c| !c.is_whitespace()).collect();
        if key.is_empty() {
            return Err(KeyError::Empty);
        }
        if !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(KeyError::InvalidChars);
        }
        if key.chars().count() < MIN_KEY_LEN {
            return Err(KeyError::TooShort);
        }
        Ok(Self(key))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// Versión enmascarada para la UI (`••••` + 4 últimos).
    pub(crate) fn masked(&self) -> String {
        mask(&self.0)
    }
}

impl std::fmt::Debug for GeminiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GeminiKey({})", self.masked())
    }
}

/// Enmascara `s`: `••••` + sus 4 últimos caracteres (solo `••••` si es tan
/// corto que revelaría demasiado).
pub(crate) fn mask(s: &str) -> String {
    let n = s.chars().count();
    if n == 0 {
        return String::new();
    }
    if n < 8 {
        return "••••".to_string();
    }
    let tail: String = s.chars().skip(n - 4).collect();
    format!("••••{tail}")
}

/// Etiqueta del botón del sheet: estado de la clave enmascarado.
pub(crate) fn sheet_key_label(key: Option<&GeminiKey>) -> String {
    match key {
        Some(k) => format!("Clave IA {}", k.masked()),
        None => "Añadir clave IA".to_string(),
    }
}

/// Ruta del fichero de la clave dentro del directorio interno.
pub(crate) fn key_path(internal_dir: &Path) -> PathBuf {
    internal_dir.join("gemini_api_key")
}

/// Carga la clave guardada (None si no hay, no se puede leer o no es válida).
pub(crate) fn load_key(internal_dir: Option<&Path>) -> Option<GeminiKey> {
    let text = std::fs::read_to_string(key_path(internal_dir?)).ok()?;
    GeminiKey::parse(&text).ok()
}

/// Guarda la clave (atómico, permisos 0600).
pub(crate) fn save_key(internal_dir: Option<&Path>, key: &GeminiKey) -> io::Result<()> {
    let Some(dir) = internal_dir else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "internal_data_path unavailable",
        ));
    };
    let path = key_path(dir);
    write_atomic(&path, key.as_str())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Borra la clave guardada (no es error si no existía).
pub(crate) fn clear_key(internal_dir: Option<&Path>) -> io::Result<()> {
    let Some(dir) = internal_dir else {
        return Ok(());
    };
    match std::fs::remove_file(key_path(dir)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    const SAMPLE: &str = "AIzaTESTONLY_not-a-real-key-0123456789";

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pr02-key-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_trims_pasted_whitespace_and_validates() {
        let k = GeminiKey::parse(&format!("  {SAMPLE}\n")).unwrap();
        assert_eq!(k.as_str(), SAMPLE);
        let split = format!("{}\n{}", &SAMPLE[..10], &SAMPLE[10..]);
        assert_eq!(GeminiKey::parse(&split).unwrap().as_str(), SAMPLE);
        assert_eq!(GeminiKey::parse("   \n").unwrap_err(), KeyError::Empty);
        assert_eq!(GeminiKey::parse("short").unwrap_err(), KeyError::TooShort);
        assert_eq!(
            GeminiKey::parse("AIza$$$$not-valid-but-long-enough").unwrap_err(),
            KeyError::InvalidChars
        );
        for e in [KeyError::Empty, KeyError::TooShort, KeyError::InvalidChars] {
            assert!(!e.message().is_empty(), "{e:?}");
        }
    }

    #[test]
    fn mask_reveals_only_the_last_four_chars() {
        let k = GeminiKey::parse(SAMPLE).unwrap();
        assert_eq!(k.masked(), "••••6789");
        assert_eq!(mask(SAMPLE), "••••6789");
        assert_eq!(mask("abc"), "••••");
        assert_eq!(mask(""), "");
        // Debug (logs, panics) nunca contiene la clave.
        let dbg = format!("{k:?}");
        assert!(!dbg.contains(SAMPLE), "{dbg}");
        assert!(!dbg.contains("AIza"), "{dbg}");
    }

    #[test]
    fn sheet_label_shows_masked_state() {
        let k = GeminiKey::parse(SAMPLE).unwrap();
        let with = sheet_key_label(Some(&k));
        assert!(with.contains("••••6789"), "{with}");
        assert!(!with.contains("AIza"), "{with}");
        let without = sheet_key_label(None);
        assert!(!without.is_empty());
        assert!(!without.contains('•'), "{without}");
    }

    #[test]
    fn key_storage_round_trip_and_clear() {
        let dir = tmp_dir("rt");
        assert!(
            load_key(Some(&dir)).is_none(),
            "instalación nueva: sin clave"
        );
        let k = GeminiKey::parse(SAMPLE).unwrap();
        save_key(Some(&dir), &k).unwrap();
        assert_eq!(load_key(Some(&dir)), Some(k.clone()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(key_path(&dir))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{mode:o}");
        }
        // Cambiar la clave sustituye la anterior.
        let k2 = GeminiKey::parse("AIzaOTHER_TESTONLY_key_abcdefghijkl").unwrap();
        save_key(Some(&dir), &k2).unwrap();
        assert_eq!(load_key(Some(&dir)), Some(k2));
        clear_key(Some(&dir)).unwrap();
        assert!(load_key(Some(&dir)).is_none());
        assert!(!key_path(&dir).exists());
        // Borrar sin clave no es error; sin directorio interno tampoco.
        clear_key(Some(&dir)).unwrap();
        assert!(load_key(None).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_key_file_loads_as_missing() {
        let dir = tmp_dir("bad");
        std::fs::write(key_path(&dir), "no vale").unwrap();
        assert!(load_key(Some(&dir)).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_key_message_points_to_settings_not_the_build_file() {
        assert!(MISSING_KEY_MSG.contains("Ajustes"), "{MISSING_KEY_MSG}");
        assert!(
            MISSING_KEY_MSG.contains("Clave de Gemini"),
            "{MISSING_KEY_MSG}"
        );
        assert!(!MISSING_KEY_MSG.contains("google_key.txt"));
        assert!(!MISSING_KEY_MSG.contains("reinstala"));
    }
}
