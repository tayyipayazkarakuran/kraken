use std::collections::HashMap;
use std::ptr;

use super::api::{Ptr, api};
use super::meta::{Class, Field, Type};
use super::value::{get_i64, get_ref};

const ARRAY_DATA: usize = 4 * size_of::<usize>();
const ARRAY_LENGTH: usize = 3 * size_of::<usize>();

pub fn array_len(array: Ptr) -> usize {
    if !super::guard::pointer(array.cast()) || !super::guard::readable(array.cast(), ARRAY_DATA) {
        return 0;
    }
    unsafe { ptr::read_unaligned(array.cast::<u8>().add(ARRAY_LENGTH).cast::<usize>()) }
}

pub fn array_data(array: Ptr) -> *const u8 {
    unsafe { array.cast::<u8>().add(ARRAY_DATA) }
}

#[derive(Clone, Copy)]
pub struct Slot {
    pub at: *const u8,
    pub ty: Type,
}

impl Slot {
    pub fn object(self) -> Ptr {
        if self.ty.prim().is_some() || self.ty.is_value() {
            return ptr::null_mut();
        }
        unsafe { ptr::read_unaligned(self.at.cast::<Ptr>()) }
    }

    pub fn i64(self) -> Option<i64> {
        let prim = self.ty.prim()?;
        Some(unsafe { prim.read_i64(self.at) })
    }

    pub fn f64(self) -> Option<f64> {
        let prim = self.ty.prim()?;
        Some(unsafe { prim.read_f64(self.at) })
    }
}

fn element_size(element: Class) -> Option<usize> {
    let size = unsafe { (api().il2cpp_class_array_element_size)(element.0) };
    (size > 0).then_some(size as usize)
}

#[derive(Clone, Copy)]
struct DictLayout {
    entries: Field,
    count: Field,
    stride: usize,
    hash: usize,
    key: usize,
    value: usize,
    key_ty: Type,
    value_ty: Type,
}

#[derive(Clone, Copy)]
struct ListLayout {
    items: Field,
    size: Field,
    stride: usize,
    item_ty: Type,
}

#[derive(Clone, Copy)]
struct SetLayout {
    slots: Field,
    count: Field,
    stride: usize,
    hash: usize,
    value: usize,
    value_ty: Type,
}

#[derive(Default)]
pub struct Collections {
    inner: HashMap<Class, Option<Field>>,
    dicts: HashMap<Class, Option<DictLayout>>,
    lists: HashMap<Class, Option<ListLayout>>,
    sets: HashMap<Class, Option<SetLayout>>,
}

unsafe impl Send for Collections {}

impl Collections {
    fn unwrap(&mut self, object: Ptr) -> Ptr {
        let Some(class) = Class::of(object) else {
            return object;
        };
        let field = *self.inner.entry(class).or_insert_with(|| {
            class.name().starts_with("ReactiveDictionary").then(|| {
                class.lineage().flat_map(Class::fields).find(|f| {
                    !f.is_static() && f.ty().name().starts_with("System.Collections.Generic.Dictionary")
                })
            })?
        });
        match field {
            Some(f) => unsafe { get_ref(object, f) },
            None => object,
        }
    }

    fn dict_layout(&mut self, class: Class) -> Option<DictLayout> {
        *self.dicts.entry(class).or_insert_with(|| {
            let entries = class.instance_field("entries")?;
            let count = class.instance_field("count")?;
            let array = entries.ty().class()?;
            let entry = Class::from_ptr(unsafe { (api().il2cpp_class_get_element_class)(array.0) })?;
            let stride = element_size(entry)?;
            let hash = entry.instance_field("hashcode")?;
            let key = entry.instance_field("key")?;
            let value = entry.instance_field("value")?;
            tracing::info!(
                dict = %class.full_name(),
                entry = %entry.full_name(),
                stride,
                hash = hash.offset(),
                key = key.offset(),
                value = value.offset(),
                key_ty = %key.ty().name(),
                value_ty = %value.ty().name(),
                "dictionary layout"
            );
            Some(DictLayout {
                entries,
                count,
                stride,
                hash: hash.inline_offset(),
                key: key.inline_offset(),
                value: value.inline_offset(),
                key_ty: key.ty(),
                value_ty: value.ty(),
            })
        })
    }

    pub fn dict(&mut self, object: Ptr) -> Vec<(Slot, Slot)> {
        let object = self.unwrap(object);
        let Some(class) = Class::of(object) else {
            return Vec::new();
        };
        let Some(layout) = self.dict_layout(class) else {
            return Vec::new();
        };
        let entries = unsafe { get_ref(object, layout.entries) };
        let count = unsafe { get_i64(object, layout.count) }.unwrap_or(0).max(0) as usize;
        let count = count.min(array_len(entries));
        let data = array_data(entries);
        if !super::guard::readable(data, count * layout.stride) {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(count);
        for index in 0..count {
            let entry = unsafe { data.add(index * layout.stride) };
            let hash = unsafe { ptr::read_unaligned(entry.add(layout.hash).cast::<i32>()) };
            if hash < 0 {
                continue;
            }
            out.push((
                Slot {
                    at: unsafe { entry.add(layout.key) },
                    ty: layout.key_ty,
                },
                Slot {
                    at: unsafe { entry.add(layout.value) },
                    ty: layout.value_ty,
                },
            ));
        }
        out
    }

    fn list_layout(&mut self, class: Class) -> Option<ListLayout> {
        *self.lists.entry(class).or_insert_with(|| {
            let items = class.instance_field("items")?;
            let size = class.instance_field("size")?;
            let array = items.ty().class()?;
            let element = Class::from_ptr(unsafe { (api().il2cpp_class_get_element_class)(array.0) })?;
            let stride = element_size(element)?;
            Some(ListLayout {
                items,
                size,
                stride,
                item_ty: element.ty(),
            })
        })
    }

    pub fn list(&mut self, object: Ptr) -> Vec<Slot> {
        let Some(class) = Class::of(object) else {
            return Vec::new();
        };
        let Some(layout) = self.list_layout(class) else {
            return Vec::new();
        };
        let items = unsafe { get_ref(object, layout.items) };
        let size =
            (unsafe { get_i64(object, layout.size) }.unwrap_or(0).max(0) as usize).min(array_len(items));
        let data = array_data(items);
        if !super::guard::readable(data, size * layout.stride) {
            return Vec::new();
        }
        (0..size)
            .map(|i| Slot {
                at: unsafe { data.add(i * layout.stride) },
                ty: layout.item_ty,
            })
            .collect()
    }

    /// `HashSet<T>`: an array of `{hashCode, next, value}` slots; free slots
    /// carry a negative hash. Mono names the array `_slots` and the high
    /// water mark `_lastIndex`, newer runtimes `_entries` and `_count`.
    fn set_layout(&mut self, class: Class) -> Option<SetLayout> {
        *self.sets.entry(class).or_insert_with(|| {
            let slots = class
                .instance_field("slots")
                .or_else(|| class.instance_field("entries"))?;
            let count = class
                .instance_field("lastindex")
                .or_else(|| class.instance_field("count"))?;
            let array = slots.ty().class()?;
            let entry = Class::from_ptr(unsafe { (api().il2cpp_class_get_element_class)(array.0) })?;
            let stride = element_size(entry)?;
            let hash = entry.instance_field("hashcode")?;
            let value = entry.instance_field("value")?;
            Some(SetLayout {
                slots,
                count,
                stride,
                hash: hash.inline_offset(),
                value: value.inline_offset(),
                value_ty: value.ty(),
            })
        })
    }

    pub fn set(&mut self, object: Ptr) -> Vec<Slot> {
        let Some(class) = Class::of(object) else {
            return Vec::new();
        };
        let Some(layout) = self.set_layout(class) else {
            return Vec::new();
        };
        let slots = unsafe { get_ref(object, layout.slots) };
        let count =
            (unsafe { get_i64(object, layout.count) }.unwrap_or(0).max(0) as usize).min(array_len(slots));
        let data = array_data(slots);
        if !super::guard::readable(data, count * layout.stride) {
            return Vec::new();
        }
        (0..count)
            .filter_map(|index| {
                let entry = unsafe { data.add(index * layout.stride) };
                let hash = unsafe { ptr::read_unaligned(entry.add(layout.hash).cast::<i32>()) };
                (hash >= 0).then(|| Slot {
                    at: unsafe { entry.add(layout.value) },
                    ty: layout.value_ty,
                })
            })
            .collect()
    }
}
