use std::collections::HashMap;
use std::ffi::CStr;

use super::api::{Ptr, api};
use super::meta::Class;

pub struct Registry {
    by_full: HashMap<String, Vec<Class>>,
    by_name: HashMap<&'static str, Vec<Class>>,
    image_of: HashMap<Class, &'static str>,
}

impl Registry {
    pub fn build() -> Self {
        let api = api();
        let mut registry = Self {
            by_full: HashMap::new(),
            by_name: HashMap::new(),
            image_of: HashMap::new(),
        };
        unsafe {
            let domain = (api.il2cpp_domain_get)();
            let mut count = 0usize;
            let assemblies = (api.il2cpp_domain_get_assemblies)(domain, &mut count);
            for assembly in std::slice::from_raw_parts(assemblies, count) {
                let image = (api.il2cpp_assembly_get_image)(*assembly);
                let image_name = image_name(image);
                for index in 0..(api.il2cpp_image_get_class_count)(image) {
                    let Some(class) = Class::from_ptr((api.il2cpp_image_get_class)(image, index)) else {
                        continue;
                    };
                    registry.by_full.entry(class.full_name()).or_default().push(class);
                    registry.by_name.entry(class.name()).or_default().push(class);
                    registry.image_of.insert(class, image_name);
                }
            }
        }
        registry
    }

    pub fn len(&self) -> usize {
        self.image_of.len()
    }

    pub fn is_empty(&self) -> bool {
        self.image_of.is_empty()
    }

    pub fn image(&self, class: Class) -> &'static str {
        self.image_of.get(&class).copied().unwrap_or("")
    }

    pub fn find(&self, name: &str, image: Option<&str>) -> Option<Class> {
        let candidates = if name.contains('.') {
            self.by_full.get(name)
        } else {
            self.by_name.get(name)
        }?;
        let mut matches = candidates.iter().copied().filter(|c| {
            image.is_none_or(|i| self.image(*c).trim_end_matches(".dll") == i.trim_end_matches(".dll"))
        });
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }

    pub fn classes_in<'a>(&'a self, image: &'a str) -> impl Iterator<Item = Class> + 'a {
        self.image_of
            .iter()
            .filter(move |(_, i)| i.trim_end_matches(".dll") == image)
            .map(|(c, _)| *c)
    }
}

fn image_name(image: Ptr) -> &'static str {
    let raw = unsafe { (api().il2cpp_image_get_name)(image) };
    if raw.is_null() {
        return "";
    }
    unsafe { CStr::from_ptr(raw) }.to_str().unwrap_or("")
}
