//! Minimal reader for Unity's `UnityFS` asset bundle container.

use super::reader::{Reader, fail};

const FLAG_COMPRESSION: u32 = 0x3f;
const FLAG_INFO_AT_END: u32 = 0x80;
const FLAG_INFO_PADDING: u32 = 0x200;

pub struct Node {
    pub path: String,
    pub data: Vec<u8>,
}

fn decompress(kind: u32, input: &[u8], size: usize) -> Result<Vec<u8>, String> {
    match kind {
        0 => Ok(input.to_vec()),
        2 | 3 => lz4_flex::block::decompress(input, size).map_err(|e| format!("lz4: {e}")),
        other => fail(format!("unsupported bundle compression {other}")),
    }
}

pub fn open(bytes: &[u8]) -> Result<Vec<Node>, String> {
    let mut r = Reader::big(bytes);
    if r.cstr()? != "UnityFS" {
        return fail("not a UnityFS bundle");
    }
    let format = r.u32()?;
    let _player = r.cstr()?;
    let _engine = r.cstr()?;
    let _size = r.i64()?;
    let info_packed = r.u32()? as usize;
    let info_size = r.u32()? as usize;
    let flags = r.u32()?;
    if format >= 7 {
        r.align(16);
    }
    let info_bytes = if flags & FLAG_INFO_AT_END != 0 {
        bytes
            .get(bytes.len().saturating_sub(info_packed)..)
            .ok_or("truncated block info")?
    } else {
        r.take(info_packed)?
    };
    let info = decompress(flags & FLAG_COMPRESSION, info_bytes, info_size)?;
    if flags & FLAG_INFO_PADDING != 0 {
        r.align(16);
    }

    let mut ir = Reader::big(&info);
    ir.skip(16)?;
    let block_count = ir.u32()? as usize;
    let mut stream = Vec::new();
    for _ in 0..block_count {
        let size = ir.u32()? as usize;
        let packed = ir.u32()? as usize;
        let block_flags = u32::from(ir.u16()?);
        stream.extend(decompress(block_flags & FLAG_COMPRESSION, r.take(packed)?, size)?);
    }
    let node_count = ir.u32()? as usize;
    let mut nodes = Vec::with_capacity(node_count);
    for _ in 0..node_count {
        let offset = usize::try_from(ir.i64()?).map_err(|e| e.to_string())?;
        let size = usize::try_from(ir.i64()?).map_err(|e| e.to_string())?;
        let _flags = ir.u32()?;
        let path = ir.cstr()?;
        let data = stream
            .get(offset..offset + size)
            .ok_or("bundle node outside data")?
            .to_vec();
        nodes.push(Node { path, data });
    }
    Ok(nodes)
}
