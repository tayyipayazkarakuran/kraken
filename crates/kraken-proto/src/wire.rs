use std::io::{self, Read, Write};

use serde::{Serialize, de::DeserializeOwned};

/// Large enough for a full font atlas upload.
pub const MAX_FRAME: usize = 32 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("codec: {0}")]
    Codec(#[from] postcard::Error),
    #[error("frame of {0} bytes exceeds limit")]
    TooLarge(usize),
}

pub fn encode_frame<T: Serialize>(value: &T) -> Result<Vec<u8>, WireError> {
    let body = postcard::to_allocvec(value)?;
    if body.len() > MAX_FRAME {
        return Err(WireError::TooLarge(body.len()));
    }
    let mut frame = Vec::with_capacity(body.len() + 4);
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

pub fn write_frame<T: Serialize, W: Write>(writer: &mut W, value: &T) -> Result<(), WireError> {
    writer.write_all(&encode_frame(value)?)?;
    writer.flush()?;
    Ok(())
}

pub fn read_frame<T: DeserializeOwned, R: Read>(reader: &mut R) -> Result<T, WireError> {
    let mut len = [0u8; 4];
    reader.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(WireError::TooLarge(len));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;
    Ok(postcard::from_bytes(&body)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentMessage, AppMessage, Config, Control, Event, EventCode, Level, Telemetry};

    #[test]
    fn round_trips_messages() {
        let mut buf = Vec::new();
        let sent = [
            AppMessage::Hello {
                version: crate::PROTO_VERSION,
            },
            AppMessage::Configure(Box::default()),
            AppMessage::Control(Control::Start),
        ];
        for m in &sent {
            write_frame(&mut buf, m).unwrap();
        }
        let mut cursor = io::Cursor::new(buf);
        for m in &sent {
            assert_eq!(&read_frame::<AppMessage, _>(&mut cursor).unwrap(), m);
        }
    }

    #[test]
    fn round_trips_agent_messages() {
        let mut buf = Vec::new();
        let telemetry = AgentMessage::Telemetry(Box::new(Telemetry {
            frame_us: 42,
            ..Default::default()
        }));
        let event = AgentMessage::Event(Event::new(Level::Warning, EventCode::Stuck).arg(3));
        write_frame(&mut buf, &telemetry).unwrap();
        write_frame(&mut buf, &event).unwrap();
        let mut cursor = io::Cursor::new(buf);
        assert_eq!(read_frame::<AgentMessage, _>(&mut cursor).unwrap(), telemetry);
        assert_eq!(read_frame::<AgentMessage, _>(&mut cursor).unwrap(), event);
    }

    #[test]
    fn rejects_oversized_frame() {
        let mut buf = ((MAX_FRAME + 1) as u32).to_le_bytes().to_vec();
        buf.extend_from_slice(&[0; 8]);
        let err = read_frame::<Config, _>(&mut io::Cursor::new(buf)).unwrap_err();
        assert!(matches!(err, WireError::TooLarge(_)));
    }
}
