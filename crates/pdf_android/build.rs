// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda
//
// Genera `keys.rs` en OUT_DIR con las claves de API de la IA (Groq/Gemini).
// Prioridad: variable de entorno > fichero en crates/pdf_android/.
//
// - Si hay claves (GROQ_API_KEY / GOOGLE_API_KEY o groq_key.txt /
//   google_key.txt, ambos gitignored), se incrustan en el binario.
// - Si no hay claves, se generan vacías: la app compila igual y la IA queda
//   deshabilitada en runtime (los clientes de pdf_core devuelven error de
//   configuración en lugar de romper el build). Un clon limpio compila sin
//   necesidad de ficheros locales.

use std::{env, fs, path::Path};

fn read_key(env_var: &str, file_name: &str) -> String {
    env::var(env_var)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|| read_key_from_file(file_name))
}

fn read_key_from_file(file_name: &str) -> String {
    let manifest = env::var("CARGO_MANIFEST_DIR").unwrap();
    let file = Path::new(&manifest).join(file_name);
    fs::read_to_string(file)
        .map(|v| v.trim().to_string())
        .unwrap_or_default()
}

fn main() {
    let out = env::var("OUT_DIR").unwrap();
    let keys = format!(
        "pub(crate) const GROQ_API_KEY: &str = {:?};\n\
         pub(crate) const GOOGLE_API_KEY: &str = {:?};\n",
        read_key("GROQ_API_KEY", "groq_key.txt"),
        read_key("GOOGLE_API_KEY", "google_key.txt"),
    );
    fs::write(Path::new(&out).join("keys.rs"), keys).unwrap();

    println!("cargo:rerun-if-changed=groq_key.txt");
    println!("cargo:rerun-if-changed=google_key.txt");
    println!("cargo:rerun-if-env-changed=GROQ_API_KEY");
    println!("cargo:rerun-if-env-changed=GOOGLE_API_KEY");
}
