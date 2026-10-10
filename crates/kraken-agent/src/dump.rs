use std::fmt::Write as _;
use std::path::PathBuf;

use crate::il2cpp::{Class, Registry};

pub fn data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join("Kraken")
}

fn describe(out: &mut String, class: Class, base: usize) {
    let _ = writeln!(out, "class {}", class.full_name());
    if let Some(parent) = class.parent() {
        let _ = writeln!(out, "  : {}", parent.full_name());
    }
    for field in class.fields() {
        let scope = if field.is_static() { "static " } else { "" };
        let _ = writeln!(
            out,
            "  {scope}{} : {} @{}",
            field.name(),
            field.ty().name(),
            field.offset()
        );
    }
    for method in class.methods() {
        let params: Vec<String> = (0..method.param_count())
            .map(|i| method.param(i).name())
            .collect();
        let code = method.code() as usize;
        let rva = if code > base {
            format!(" rva=0x{:x}", code - base)
        } else {
            String::new()
        };
        let _ = writeln!(out, "  {}({}){rva}", method.name(), params.join(", "));
    }
}

pub fn write(registry: &Registry) -> Result<PathBuf, String> {
    let dir = data_dir().join("dumps");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let base = crate::il2cpp::game_module().map_or(0, |m| m as usize);
    let mut out = format!(
        "# GameAssembly base 0x{base:x}
"
    );
    for image in ["Seafight", "libcom-protocol", "CakeBox"] {
        let mut classes: Vec<Class> = registry.classes_in(image).collect();
        classes.sort_by_key(|c| c.full_name());
        let _ = writeln!(out, "# {image} ({} classes)", classes.len());
        for class in classes {
            describe(&mut out, class, base);
        }
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let path = dir.join(format!("types-{stamp}.txt"));
    std::fs::write(&path, out).map_err(|e| e.to_string())?;
    Ok(path)
}
