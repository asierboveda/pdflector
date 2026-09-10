// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Lista curada de ~45 categorías populares de arXiv con descripciones y etiquetas en español.
//!
//! Permite al usuario filtrar el feed de Discover por sus áreas de interés y persistir
//! la selección en `discover.json` (`DiscoverPrefs`).

/// Categoría de arXiv con su código canónico, etiqueta legible en español y grupo principal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArxivCategory {
    pub code: &'static str,
    pub label_es: &'static str,
    pub group: &'static str,
}

/// Catálogo de ~45 categorías de arXiv relevantes para investigación y lectura técnica.
pub const ARXIV_CATEGORIES: &[ArxivCategory] = &[
    // --- Computer Science (cs) ---
    ArxivCategory {
        code: "cs.AI",
        label_es: "Inteligencia Artificial",
        group: "Informática",
    },
    ArxivCategory {
        code: "cs.CL",
        label_es: "Computación y Lenguaje (NLP)",
        group: "Informática",
    },
    ArxivCategory {
        code: "cs.CV",
        label_es: "Visión por Computador",
        group: "Informática",
    },
    ArxivCategory {
        code: "cs.LG",
        label_es: "Aprendizaje Automático (ML)",
        group: "Informática",
    },
    ArxivCategory {
        code: "cs.CR",
        label_es: "Criptografía y Seguridad",
        group: "Informática",
    },
    ArxivCategory {
        code: "cs.DB",
        label_es: "Bases de Datos",
        group: "Informática",
    },
    ArxivCategory {
        code: "cs.DC",
        label_es: "Sistemas Distribuidos y Paralelos",
        group: "Informática",
    },
    ArxivCategory {
        code: "cs.DS",
        label_es: "Estructuras de Datos y Algoritmos",
        group: "Informática",
    },
    ArxivCategory {
        code: "cs.HC",
        label_es: "Interacción Persona-Ordenador (HCI)",
        group: "Informática",
    },
    ArxivCategory {
        code: "cs.IR",
        label_es: "Recuperación de Información (IR)",
        group: "Informática",
    },
    ArxivCategory {
        code: "cs.IT",
        label_es: "Teoría de la Información",
        group: "Informática",
    },
    ArxivCategory {
        code: "cs.NE",
        label_es: "Redes Neuronales y Evolución",
        group: "Informática",
    },
    ArxivCategory {
        code: "cs.PL",
        label_es: "Lenguajes de Programación",
        group: "Informática",
    },
    ArxivCategory {
        code: "cs.RO",
        label_es: "Robótica",
        group: "Informática",
    },
    ArxivCategory {
        code: "cs.SE",
        label_es: "Ingeniería de Software",
        group: "Informática",
    },
    ArxivCategory {
        code: "cs.SI",
        label_es: "Redes Sociales y de Información",
        group: "Informática",
    },
    // --- Mathematics (math) ---
    ArxivCategory {
        code: "math.AG",
        label_es: "Geometría Algebraica",
        group: "Matemáticas",
    },
    ArxivCategory {
        code: "math.AP",
        label_es: "Análisis de EDPs",
        group: "Matemáticas",
    },
    ArxivCategory {
        code: "math.CO",
        label_es: "Combinatoria",
        group: "Matemáticas",
    },
    ArxivCategory {
        code: "math.DS",
        label_es: "Sistemas Dinámicos",
        group: "Matemáticas",
    },
    ArxivCategory {
        code: "math.NT",
        label_es: "Teoría de Números",
        group: "Matemáticas",
    },
    ArxivCategory {
        code: "math.OC",
        label_es: "Optimización y Control",
        group: "Matemáticas",
    },
    ArxivCategory {
        code: "math.PR",
        label_es: "Probabilidad",
        group: "Matemáticas",
    },
    ArxivCategory {
        code: "math.ST",
        label_es: "Teoría Estadística",
        group: "Matemáticas",
    },
    // --- Physics (physics / astro / hep / quant-ph) ---
    ArxivCategory {
        code: "quant-ph",
        label_es: "Física Cuántica",
        group: "Física",
    },
    ArxivCategory {
        code: "hep-th",
        label_es: "Teoría de Altas Energías",
        group: "Física",
    },
    ArxivCategory {
        code: "hep-ph",
        label_es: "Fenomenología de Altas Energías",
        group: "Física",
    },
    ArxivCategory {
        code: "cond-mat.mes-hall",
        label_es: "Materia Condensada (Mesoscópica)",
        group: "Física",
    },
    ArxivCategory {
        code: "cond-mat.mtrl-sci",
        label_es: "Ciencia de Materiales",
        group: "Física",
    },
    ArxivCategory {
        code: "cond-mat.stat-mech",
        label_es: "Mecánica Estadística",
        group: "Física",
    },
    ArxivCategory {
        code: "cond-mat.str-el",
        label_es: "Electrones Fuertemente Correlacionados",
        group: "Física",
    },
    ArxivCategory {
        code: "astro-ph.CO",
        label_es: "Cosmología y Astrofísica Galáctica",
        group: "Física",
    },
    ArxivCategory {
        code: "astro-ph.EP",
        label_es: "Astrofísica de Planetas y Exoplanetas",
        group: "Física",
    },
    ArxivCategory {
        code: "gr-qc",
        label_es: "Relatividad General y Cuántica",
        group: "Física",
    },
    ArxivCategory {
        code: "physics.soc-ph",
        label_es: "Física y Sociedad",
        group: "Física",
    },
    ArxivCategory {
        code: "physics.bio-ph",
        label_es: "Física Biológica",
        group: "Física",
    },
    // --- Statistics (stat) ---
    ArxivCategory {
        code: "stat.ML",
        label_es: "Aprendizaje Automático Estadístico",
        group: "Estadística",
    },
    ArxivCategory {
        code: "stat.ME",
        label_es: "Metodología Estadística",
        group: "Estadística",
    },
    ArxivCategory {
        code: "stat.AP",
        label_es: "Estadística Aplicada",
        group: "Estadística",
    },
    // --- Quantitative Biology (q-bio) ---
    ArxivCategory {
        code: "q-bio.NC",
        label_es: "Neuronas y Cognición",
        group: "Biología Cuantitativa",
    },
    ArxivCategory {
        code: "q-bio.QM",
        label_es: "Métodos Cuantitativos en Biología",
        group: "Biología Cuantitativa",
    },
    ArxivCategory {
        code: "q-bio.GN",
        label_es: "Genómica y Genética",
        group: "Biología Cuantitativa",
    },
    // --- Quantitative Finance (q-fin) ---
    ArxivCategory {
        code: "q-fin.CP",
        label_es: "Finanzas Computacionales",
        group: "Finanzas Cuantitativas",
    },
    ArxivCategory {
        code: "q-fin.PM",
        label_es: "Gestión de Carteras",
        group: "Finanzas Cuantitativas",
    },
    ArxivCategory {
        code: "q-fin.ST",
        label_es: "Finanzas Estadísticas",
        group: "Finanzas Cuantitativas",
    },
    // --- Electrical Engineering and Systems Science (eess) ---
    ArxivCategory {
        code: "eess.SP",
        label_es: "Procesamiento de Señales",
        group: "Ingeniería Eléctrica",
    },
    ArxivCategory {
        code: "eess.IV",
        label_es: "Procesamiento de Imagen y Vídeo",
        group: "Ingeniería Eléctrica",
    },
    ArxivCategory {
        code: "eess.SY",
        label_es: "Sistemas y Control",
        group: "Ingeniería Eléctrica",
    },
];

/// Devuelve la etiqueta legible en español para un código de categoría, o el código mismo si no se encuentra.
pub fn category_label(code: &str) -> &str {
    for cat in ARXIV_CATEGORIES {
        if cat.code.eq_ignore_ascii_case(code) {
            return cat.label_es;
        }
    }
    code
}

/// Categorías activas por defecto para nuevos usuarios si no hay preferencias guardadas.
pub fn default_categories() -> Vec<String> {
    vec!["cs.AI".to_string(), "cs.LG".to_string()]
}
