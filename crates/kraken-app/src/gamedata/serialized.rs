//! Reads MonoBehaviour objects out of a Unity `SerializedFile` by walking the
//! embedded type tree, so field layout changes in game updates are tolerated.

use super::reader::{Reader, fail};

const MONO_BEHAVIOUR: i32 = 114;
const ALIGN_AFTER: i32 = 0x4000;
const COMMON_STRING: u32 = 0x8000_0000;
const MAX_DEPTH: usize = 32;

struct TypeNode {
    level: u8,
    is_array: bool,
    name: String,
    size: i32,
    meta: i32,
}

struct Type {
    class_id: i32,
    nodes: Vec<TypeNode>,
}

pub enum Value {
    Scalar,
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Struct(Vec<(String, Value)>),
}

impl Value {
    pub fn field(&self, name: &str) -> Option<&Self> {
        match self {
            Self::Struct(fields) => fields.iter().find(|(n, _)| n == name).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Array contents, looking through the wrapper struct Unity puts around
    /// serialized lists.
    pub fn items(&self) -> Option<&[Self]> {
        match self {
            Self::List(items) => Some(items),
            Self::Struct(fields) if fields.len() == 1 => fields[0].1.items(),
            _ => None,
        }
    }

    pub fn text(&self) -> Option<String> {
        match self {
            Self::Bytes(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
            Self::Struct(fields) => fields.first()?.1.text(),
            _ => None,
        }
    }
}

fn read_tree(r: &mut Reader<'_>, version: u32) -> Result<Vec<TypeNode>, String> {
    let count = r.u32()? as usize;
    let strings_len = r.u32()? as usize;
    let mut raw = Vec::with_capacity(count);
    for _ in 0..count {
        let _version = r.u16()?;
        let level = r.u8()?;
        let flags = r.u8()?;
        let _type = r.u32()?;
        let name = r.u32()?;
        let size = r.i32()?;
        let _index = r.i32()?;
        let meta = r.i32()?;
        if version >= 19 {
            r.skip(8)?;
        }
        raw.push((level, flags, name, size, meta));
    }
    let strings = r.take(strings_len)?;
    let local = |offset: u32| -> String {
        if offset & COMMON_STRING != 0 {
            return String::new();
        }
        let rest = strings.get(offset as usize..).unwrap_or_default();
        let end = rest.iter().position(|b| *b == 0).unwrap_or(rest.len());
        String::from_utf8_lossy(&rest[..end]).into_owned()
    };
    Ok(raw
        .into_iter()
        .map(|(level, flags, name, size, meta)| TypeNode {
            level,
            is_array: flags & 1 != 0,
            name: local(name),
            size,
            meta,
        })
        .collect())
}

fn children(nodes: &[TypeNode], index: usize) -> Vec<usize> {
    let level = nodes[index].level;
    let mut out = Vec::new();
    for (i, node) in nodes.iter().enumerate().skip(index + 1) {
        if node.level <= level {
            break;
        }
        if node.level == level + 1 {
            out.push(i);
        }
    }
    out
}

fn read_value(r: &mut Reader<'_>, nodes: &[TypeNode], index: usize, depth: usize) -> Result<Value, String> {
    if depth > MAX_DEPTH {
        return fail("type tree too deep");
    }
    let node = &nodes[index];
    let kids = children(nodes, index);
    let value = if node.is_array {
        let [_, element] = kids.as_slice() else {
            return fail("malformed array node");
        };
        let count = usize::try_from(r.i32()?).map_err(|_| "negative array size")?;
        let item = &nodes[*element];
        if item.size == 1 && children(nodes, *element).is_empty() {
            Value::Bytes(r.take(count)?.to_vec())
        } else {
            let mut items = Vec::with_capacity(count.min(1 << 16));
            for _ in 0..count {
                items.push(read_value(r, nodes, *element, depth + 1)?);
            }
            Value::List(items)
        }
    } else if kids.is_empty() {
        let size = usize::try_from(node.size).map_err(|_| "unsized leaf")?;
        r.skip(size)?;
        Value::Scalar
    } else {
        let mut fields = Vec::with_capacity(kids.len());
        for kid in kids {
            let value = read_value(r, nodes, kid, depth + 1)?;
            fields.push((nodes[kid].name.clone(), value));
        }
        Value::Struct(fields)
    };
    if node.meta & ALIGN_AFTER != 0 {
        r.align(4);
    }
    Ok(value)
}

/// Every MonoBehaviour in the file, decoded through its type tree.
pub fn mono_behaviours(bytes: &[u8]) -> Result<Vec<Value>, String> {
    let mut r = Reader::big(bytes);
    let _metadata = r.u32()?;
    let _size = r.u32()?;
    let version = r.u32()?;
    let mut data_offset = u64::from(r.u32()?);
    if version < 17 {
        return fail(format!("serialized file version {version} is too old"));
    }
    let little = r.u8()? == 0;
    r.skip(3)?;
    if version >= 22 {
        let _metadata = r.u32()?;
        let _size = r.i64()?;
        data_offset = u64::try_from(r.i64()?).map_err(|e| e.to_string())?;
        r.skip(8)?;
    }
    r.set_little(little);
    let _engine = r.cstr()?;
    let _platform = r.i32()?;
    let type_tree = r.u8()? != 0;
    if !type_tree {
        return fail("bundle has no type tree");
    }
    let type_count = r.u32()? as usize;
    let mut types = Vec::with_capacity(type_count);
    for _ in 0..type_count {
        let class_id = r.i32()?;
        let _stripped = r.u8()?;
        let _script = r.u16()?;
        if class_id == MONO_BEHAVIOUR {
            r.skip(16)?;
        }
        r.skip(16)?;
        let nodes = read_tree(&mut r, version)?;
        if version >= 21 {
            let deps = r.u32()? as usize;
            r.skip(deps * 4)?;
        }
        types.push(Type { class_id, nodes });
    }
    let object_count = r.u32()? as usize;
    let mut out = Vec::new();
    for _ in 0..object_count {
        r.align(4);
        let _path = r.i64()?;
        let start = if version >= 22 {
            u64::try_from(r.i64()?).map_err(|e| e.to_string())?
        } else {
            u64::from(r.u32()?)
        };
        let _size = r.u32()?;
        let type_index = usize::try_from(r.i32()?).map_err(|_| "bad type index")?;
        let Some(ty) = types.get(type_index) else {
            continue;
        };
        if ty.class_id != MONO_BEHAVIOUR || ty.nodes.is_empty() {
            continue;
        }
        let at = usize::try_from(data_offset + start).map_err(|e| e.to_string())?;
        let mut data = Reader::at(bytes, at, little);
        out.push(read_value(&mut data, &ty.nodes, 0, 0)?);
    }
    Ok(out)
}
