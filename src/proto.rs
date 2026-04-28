
use std::io::{Read, Cursor, Error, ErrorKind};
use std::str;
use byteorder::{BigEndian, WriteBytesExt, ByteOrder};
use uuid::Uuid;
use std::time::{SystemTime, UNIX_EPOCH};

const SEGMENT_BITS: u32 = 0x7F;
const CONTINUE_BIT: u32 = 0x80;

// returns varint as big endian
fn encode_varint(v: i32, dst: &mut Vec<u8>) -> usize {
    let mut uv = v as u32;
    let mut written: usize = 0;
    loop {
        if uv & !SEGMENT_BITS == 0 {
            dst.push(uv as u8);
            written += 1;
            return written
        }
        dst.push(((uv & SEGMENT_BITS)|CONTINUE_BIT) as u8);
        written += 1;
        uv >>= 7;
    }
}

fn encode_string(v: &str, dst: &mut Vec<u8>) -> usize {
    let mut written = encode_varint(v.len() as i32, dst);
    dst.extend_from_slice(v.as_bytes()); // TODO written in native endian,
                                         // should be in big endian
    written += v.len();
    written
}

fn read_varint(r: &mut impl Read) -> Result<i32, Error> {
    let mut v: i32 = 0;
    let mut pos: i32 = 0;
    for byte in r.bytes() {
        let b = byte?;
        v |= ((b as u32 & SEGMENT_BITS) << pos) as i32;
        if b as u32 & CONTINUE_BIT == 0 {
            return Ok(v)
        }
        pos += 7;
        if pos >= 32 {
            return Err(Error::new(ErrorKind::Other, "varint is too big"))
        }
    }
    return Err(Error::new(ErrorKind::Other, "no data while reading varint"))
}

fn read_string(r: &mut impl Read) -> Result<String, Error> {
    let str_len = read_varint(r)? as usize;
    let mut buf: Vec<u8> = vec![0; str_len];
    let _ = r.read_exact(&mut buf[..]);
    match String::from_utf8(buf) {
        Ok(v) => return Ok(v),
        Err(e) => return Err(Error::new(ErrorKind::Other, e))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_varint_zero() {
        let mut dst = Vec::new();
        assert_eq!(encode_varint(0, &mut dst), 1);
        assert_eq!(dst, vec![0]);
    }

    #[test]
    fn encode_varint_biggest() {
        let mut dst = Vec::new();
        assert_eq!(encode_varint(2147483647, &mut dst), 5);
        assert_eq!(dst, vec![255, 255, 255, 255, 7]);
    }

    #[test]
    fn encode_varint_smallest() {
        let mut dst = Vec::new();
        assert_eq!(encode_varint(-2147483648, &mut dst), 5);
        assert_eq!(dst, vec![128, 128, 128, 128, 8]);
    }

    #[test]
    fn encode_varint_minus_one() {
        let mut dst = Vec::new();
        assert_eq!(encode_varint(-1, &mut dst), 5);
        assert_eq!(dst, vec![255, 255, 255, 255, 15]);
    }

    #[test]
    fn read_varint_zero() {
        let v: [u8; 1] = [0x00];
        assert_eq!(read_varint(&mut Cursor::new(v)).unwrap(), 0 as i32);
    }

    #[test]
    fn read_varint_biggest() {
        let v: [u8; 5] = [0xff, 0xff, 0xff, 0xff, 0x07];
        assert_eq!(read_varint(&mut Cursor::new(v)).unwrap(), 2147483647 as i32);
    }

    #[test]
    fn read_varint_minus_one() {
        let v: [u8; 5] = [0xff, 0xff, 0xff, 0xff, 0x0f];
        assert_eq!(read_varint(&mut Cursor::new(v)).unwrap(), -1 as i32);
    }

    #[test]
    fn read_varint_smallest() {
        let v: [u8; 5] = [0x80, 0x80, 0x80, 0x80, 0x08];
        assert_eq!(read_varint(&mut Cursor::new(v)).unwrap(), -2147483648 as i32);
    }
}

pub struct GameProfile {
    pub uuid: Uuid,
    pub username: String,
    pub props: Vec<u8>,
}

impl GameProfile {
    pub fn read_from(src: &mut impl Read) -> Result<Self, Error> {
        let mut uuid_bytes: [u8; 16] = [0; 16];
        let _ = src.read_exact(&mut uuid_bytes)?;
        let username = read_string(src)?;
        // next is prefixed array: length + data
        let pref_len = read_varint(src)? as usize;
        let mut arr_data: Vec<u8> = vec![0; pref_len];
        let _ = src.read_exact(&mut arr_data[..])?;
        Ok(GameProfile{
            uuid: Uuid::from_bytes(uuid_bytes),
            username: username,
            props: arr_data,
        })
    }
}

#[derive(Debug)]
pub enum ConnectionState {
    Handshaking,
    Status,
    Login,
}

pub enum Packet {
    StatusResponse { data: String },
    PongResponse { start_timestamp: i64, stop_timestamp: i64 },
    EncryptionRequest, // TODO
    LoginDisconnect { reason: String },
    SetCompression { threshold: i32 }, // negative value disables compression
    LoginSuccess { game_profile: GameProfile },
}

impl Packet {
    pub fn read_from(src: &mut impl Read, state: ConnectionState, compression: i32) -> Result<Packet, Error> {
        let packet_len = read_varint(src)? as usize;
        let mut packet_data: Vec<u8> = vec![0; packet_len];
        src.read_exact(&mut packet_data[..])?;
        let mut cur = Cursor::new(&packet_data);
        let packet_id: i32;
        if compression > 0 {
            let data_len = read_varint(&mut cur)? as usize;
            if data_len > 0 {
                // TODO: data is compressed, uncompress it
            }
            packet_id = read_varint(&mut cur)?;
        } else {
            packet_id = read_varint(&mut cur)?;
        }
        match packet_id {
            0x00 => match state {
                ConnectionState::Status => {
                    let resp_str = read_string(&mut cur)?;
                    return Ok(Packet::StatusResponse{ data: resp_str })
                }
                ConnectionState::Login => {
                    Ok(Packet::LoginDisconnect{
                        reason: String::from_utf8_lossy(
                            &packet_data[cur.position() as usize..]
                        ).into_owned()
                    })
                },
                _ => unreachable!(),
            },
            0x01 => match state {
                ConnectionState::Status => {
                    let timestamp = BigEndian::read_i64(&packet_data[cur.position() as usize..]);
                let stop = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("error getting timestamp")
                    .as_millis() as i64;
                    Ok(Packet::PongResponse{ start_timestamp: timestamp, stop_timestamp: stop })
                },
                _ => unreachable!(),
            },
            0x02 => match state {
                ConnectionState::Login => {
                    match GameProfile::read_from(&mut cur) {
                        Ok(p) => return Ok(Packet::LoginSuccess{game_profile: p}),
                        Err(e) => return Err(Error::new(ErrorKind::Other, format!("err parsing game profile: {}", e))),
                    }
                },
                _ => unreachable!(),
            }
            0x03 => match state {
                ConnectionState::Login => {
                    let maxsize = read_varint(&mut cur)?;
                    return Ok(Packet::SetCompression{threshold: maxsize})
                },
                _ => todo!(),
            }
            _ => todo!(),
        }
    }
}

#[repr(u32)]
#[derive(Clone, Copy)]
pub enum HandshakeIntent {
    Status = 1,
    Login = 2,
    Transfer = 3,
}

pub enum ServerboundPacket<'a> {
    Handshake {
        proto_version: i32,
        server_address: String,
        server_port: u16,
        intent: HandshakeIntent,
    },
    StatusRequest,
    PingRequest,
    LoginStart {
        username: &'a str,
        uuid: Uuid,
    },
    LoginAcknowledged,
}

pub trait Encode {
    fn encode(&self) -> Result<Vec<u8>, Error>;
}

impl Encode for ServerboundPacket<'_> {
    fn encode(&self) -> Result<Vec<u8>, Error> {
        match self {
            ServerboundPacket::Handshake {
                proto_version,
                server_address,
                server_port,
                intent,
            } => {
                let mut buf: Vec<u8> = Vec::with_capacity(275);
                encode_varint(0x00, &mut buf);
                encode_varint(*proto_version, &mut buf);
                encode_string(&server_address, &mut buf); // TODO still not BigEndian but works
                                                               // now
                buf.write_u16::<BigEndian>(*server_port)?;
                encode_varint(*intent as i32, &mut buf);
                let len = encode_varint(buf.len() as i32, &mut buf);
                buf.rotate_right(len); // move length to the beginning of packet
                Ok(buf)
            },
            ServerboundPacket::StatusRequest => {
                let mut buf: Vec<u8> = Vec::new();
                encode_varint(0x00, &mut buf);
                let len = encode_varint(buf.len() as i32, &mut buf);
                buf.rotate_right(len); // move length to the beginning of packet
                Ok(buf)
            },
            ServerboundPacket::PingRequest => {
                let mut buf: Vec<u8> = Vec::new();
                encode_varint(0x01, &mut buf);
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("error getting timestamp")
                    .as_millis() as i64;
                buf.write_i64::<BigEndian>(timestamp)?;
                let len = encode_varint(buf.len() as i32, &mut buf);
                buf.rotate_right(len);
                Ok(buf)
            },
            ServerboundPacket::LoginStart {username, uuid} => {
                let mut buf: Vec<u8> = Vec::with_capacity(42);
                encode_varint(0x00, &mut buf);
                encode_string(username, &mut buf);
                buf.extend_from_slice(uuid.as_bytes());
                let len = encode_varint(buf.len() as i32, &mut buf);
                buf.rotate_right(len);
                Ok(buf)
            },
            ServerboundPacket::LoginAcknowledged => {
                let mut buf: Vec<u8> = Vec::new();
                encode_varint(0x03, &mut buf);
                let len = encode_varint(buf.len() as i32, &mut buf);
                buf.rotate_right(len);
                Ok(buf)
            },
        }
    }
}

