use std::{
    net::{SocketAddr, IpAddr},
    time::{SystemTime, Duration},
};

use radiation::{Absorb, Emit, nom, ParseError};

use super::database::{StreamKind, StreamMeta};

pub fn addr_absorb(input: &[u8]) -> nom::IResult<&[u8], SocketAddr, ParseError<&[u8]>> {
    let pair = nom::sequence::pair(<[u8; 16]>::absorb::<()>, u16::absorb::<()>);
    nom::combinator::map(pair, |(ip, port)| {
        SocketAddr::new(IpAddr::V6(ip.into()), port)
    })(input)
}

pub fn addr_emit<W>(value: &SocketAddr, buffer: W) -> W
where
    W: Extend<u8>,
{
    let ip = match value.ip() {
        IpAddr::V6(ip) => ip.octets(),
        IpAddr::V4(ip) => ip.to_ipv6_mapped().octets(),
    };
    value.port().emit(ip.emit(buffer))
}

pub fn duration_absorb(input: &[u8]) -> nom::IResult<&[u8], Duration, ParseError<&[u8]>> {
    nom::combinator::map(
        nom::sequence::pair(u64::absorb::<()>, u32::absorb::<()>),
        |(secs, nanos)| Duration::new(secs, nanos),
    )(input)
}

pub fn duration_emit<W>(value: &Duration, buffer: W) -> W
where
    W: Extend<u8>,
{
    value.subsec_nanos().emit(value.as_secs().emit(buffer))
}

pub fn time_absorb(input: &[u8]) -> nom::IResult<&[u8], SystemTime, ParseError<&[u8]>> {
    nom::combinator::map(duration_absorb, |d| SystemTime::UNIX_EPOCH + d)(input)
}

pub fn time_emit<W>(value: &SystemTime, buffer: W) -> W
where
    W: Extend<u8>,
{
    duration_emit(
        &value.duration_since(SystemTime::UNIX_EPOCH).unwrap(),
        buffer,
    )
}

pub fn stream_kind_emit<W>(value: &StreamKind, buffer: W) -> W
where
    W: Extend<u8>,
{
    (*value as u16).emit(buffer)
}

pub fn stream_kind_absorb(input: &[u8]) -> nom::IResult<&[u8], StreamKind, ParseError<&[u8]>> {
    nom::combinator::map(u16::absorb::<()>, |d| {
        for v in StreamKind::iter() {
            if d == v as u16 {
                return v;
            }
        }
        StreamKind::Unknown
    })(input)
}

pub fn stream_meta_emit<W>(value: &StreamMeta, buffer: W) -> W
where
    W: Extend<u8>,
{
    let d = match value {
        StreamMeta::Raw => i64::MAX,
        StreamMeta::Handshake => i64::MIN,
        StreamMeta::Forward(s) => *s as i64,
        StreamMeta::Backward(s) => -((*s + 1) as i64),
    };
    d.emit(buffer)
}

pub fn stream_meta_absorb(input: &[u8]) -> nom::IResult<&[u8], StreamMeta, ParseError<&[u8]>> {
    nom::combinator::map(i64::absorb::<()>, |d| match d {
        i64::MAX => StreamMeta::Raw,
        i64::MIN => StreamMeta::Handshake,
        d => {
            if d >= 0 {
                StreamMeta::Forward(d as u64)
            } else {
                StreamMeta::Backward((-d - 1) as u64)
            }
        }
    })(input)
}
