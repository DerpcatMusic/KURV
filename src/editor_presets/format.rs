use std::io::{self, Read};

use super::{invalid_data, invalid_input};

const MAGIC: [u8; 8] = *b"KURVPSET";
const VERSION: u16 = 2;
const V1_HEADER_LEN: usize = 20;
const HEADER_LEN: usize = 24;
const PARAM_LEN: usize = 12;
const MAX_NAME_BYTES: usize = 96;
const MAX_PARAMS: usize = 4_096;
const MAX_CUSTOM_STATE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct Snapshot {
    pub(super) params: Vec<(u32, f64)>,
    pub(super) custom: Vec<u8>,
    pub(super) persist: Vec<u8>,
}

pub(crate) fn sanitize_name(requested: &str) -> io::Result<String> {
    let mut name = String::new();
    let mut last_separator = false;
    for character in requested.trim().chars() {
        let allowed = character.is_alphanumeric() || matches!(character, ' ' | '-' | '_');
        let character = if allowed { character } else { '-' };
        let separator = matches!(character, ' ' | '-' | '_');
        if separator && last_separator {
            continue;
        }
        if name.len() + character.len_utf8() > MAX_NAME_BYTES {
            break;
        }
        name.push(character);
        last_separator = separator;
    }
    let trimmed = name.trim_matches(|character| matches!(character, ' ' | '-' | '_'));
    if trimmed.is_empty() {
        return Err(invalid_input("preset name is empty"));
    }
    name = String::from(trimmed);
    if is_windows_reserved(&name) {
        name.insert(0, '_');
        while name.len() > MAX_NAME_BYTES {
            name.pop();
        }
    }
    Ok(name)
}

fn is_windows_reserved(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && matches!(&upper[..3], "COM" | "LPT")
            && matches!(upper.as_bytes()[3], b'1'..=b'9'))
}

pub(super) fn encode(name: &str, snapshot: &Snapshot) -> io::Result<Vec<u8>> {
    let name_bytes = name.as_bytes();
    if name_bytes.is_empty() || name_bytes.len() > MAX_NAME_BYTES {
        return Err(invalid_input("preset name is too long"));
    }
    validate_snapshot(
        &snapshot.params,
        snapshot.custom.len(),
        snapshot.persist.len(),
    )?;
    let name_len = u16::try_from(name_bytes.len()).map_err(|_| invalid_input("name overflow"))?;
    let param_count =
        u32::try_from(snapshot.params.len()).map_err(|_| invalid_input("param count overflow"))?;
    let state_len =
        u32::try_from(snapshot.custom.len()).map_err(|_| invalid_input("custom state overflow"))?;
    let persist_len = u32::try_from(snapshot.persist.len())
        .map_err(|_| invalid_input("persist state overflow"))?;
    let param_bytes = snapshot.params.len() * PARAM_LEN;
    let mut encoded = Vec::with_capacity(
        HEADER_LEN
            + name_bytes.len()
            + param_bytes
            + snapshot.custom.len()
            + snapshot.persist.len(),
    );
    encoded.extend_from_slice(&MAGIC);
    encoded.extend_from_slice(&VERSION.to_le_bytes());
    encoded.extend_from_slice(&name_len.to_le_bytes());
    encoded.extend_from_slice(&param_count.to_le_bytes());
    encoded.extend_from_slice(&state_len.to_le_bytes());
    encoded.extend_from_slice(&persist_len.to_le_bytes());
    encoded.extend_from_slice(name_bytes);
    for (id, normalized) in &snapshot.params {
        encoded.extend_from_slice(&id.to_le_bytes());
        encoded.extend_from_slice(&normalized.to_bits().to_le_bytes());
    }
    encoded.extend_from_slice(&snapshot.custom);
    encoded.extend_from_slice(&snapshot.persist);
    Ok(encoded)
}

pub(super) fn validate_file_length(file_len: usize) -> io::Result<()> {
    if file_len < V1_HEADER_LEN
        || file_len > HEADER_LEN + MAX_NAME_BYTES + MAX_PARAMS * PARAM_LEN + MAX_CUSTOM_STATE_BYTES
    {
        return Err(invalid_data("invalid preset size"));
    }
    Ok(())
}

pub(super) fn decode_name(reader: &mut impl Read, file_len: usize) -> io::Result<String> {
    let mut prefix = [0_u8; 10];
    reader.read_exact(&mut prefix)?;
    let encoded_len = encoded_header_len(&prefix)?;
    let mut encoded = [0_u8; HEADER_LEN];
    encoded[..prefix.len()].copy_from_slice(&prefix);
    reader.read_exact(&mut encoded[prefix.len()..encoded_len])?;
    let header = decode_header(&encoded[..encoded_len])?;
    let expected = preset_length(header)?;
    if expected != file_len {
        return Err(invalid_data("preset length mismatch"));
    }
    let mut name = vec![0_u8; header.name_len];
    reader.read_exact(&mut name)?;
    let name = String::from_utf8(name).map_err(|_| invalid_data("preset name is not UTF-8"))?;
    if !sanitize_name(&name).is_ok_and(|sanitized| sanitized == name) {
        return Err(invalid_data("invalid preset name"));
    }
    Ok(name)
}

pub(super) fn decode(bytes: Vec<u8>) -> io::Result<(String, Snapshot)> {
    validate_file_length(bytes.len())?;
    let encoded_len = encoded_header_len(&bytes)?;
    let encoded = bytes
        .get(..encoded_len)
        .ok_or_else(|| invalid_data("truncated KURV preset header"))?;
    let header = decode_header(encoded)?;
    let params_start = header
        .encoded_len
        .checked_add(header.name_len)
        .ok_or_else(|| invalid_data("preset length overflow"))?;
    let state_start = params_start
        .checked_add(
            header
                .param_count
                .checked_mul(PARAM_LEN)
                .ok_or_else(|| invalid_data("preset length overflow"))?,
        )
        .ok_or_else(|| invalid_data("preset length overflow"))?;
    let persist_start = state_start
        .checked_add(header.custom_len)
        .ok_or_else(|| invalid_data("preset length overflow"))?;
    let expected = persist_start
        .checked_add(header.persist_len)
        .ok_or_else(|| invalid_data("preset length overflow"))?;
    if expected != bytes.len() {
        return Err(invalid_data("preset length mismatch"));
    }
    let name = String::from_utf8(bytes[header.encoded_len..params_start].to_vec())
        .map_err(|_| invalid_data("preset name is not UTF-8"))?;
    if !sanitize_name(&name).is_ok_and(|sanitized| sanitized == name) {
        return Err(invalid_data("invalid preset name"));
    }
    let mut params = Vec::with_capacity(header.param_count);
    for record in bytes[params_start..state_start].chunks_exact(PARAM_LEN) {
        let id = u32::from_le_bytes(record[..4].try_into().map_err(|_| invalid_data("bad ID"))?);
        let normalized = f64::from_bits(u64::from_le_bytes(
            record[4..]
                .try_into()
                .map_err(|_| invalid_data("bad value"))?,
        ));
        params.push((id, normalized));
    }
    validate_snapshot(&params, header.custom_len, header.persist_len)?;
    Ok((
        name,
        Snapshot {
            params,
            custom: bytes[state_start..persist_start].to_vec(),
            persist: bytes[persist_start..].to_vec(),
        },
    ))
}

fn encoded_header_len(prefix: &[u8]) -> io::Result<usize> {
    if prefix.len() < 10 || prefix[..8] != MAGIC {
        return Err(invalid_data("not a KURV preset"));
    }
    match u16::from_le_bytes([prefix[8], prefix[9]]) {
        1 => Ok(V1_HEADER_LEN),
        VERSION => Ok(HEADER_LEN),
        _ => Err(invalid_data("unsupported KURV preset version")),
    }
}

#[derive(Clone, Copy)]
struct PresetHeader {
    encoded_len: usize,
    name_len: usize,
    param_count: usize,
    custom_len: usize,
    persist_len: usize,
}

fn decode_header(header: &[u8]) -> io::Result<PresetHeader> {
    let encoded_len = encoded_header_len(header)?;
    if header.len() != encoded_len {
        return Err(invalid_data("invalid KURV preset header"));
    }
    let name_len = usize::from(u16::from_le_bytes([header[10], header[11]]));
    let param_count = usize::try_from(u32::from_le_bytes([
        header[12], header[13], header[14], header[15],
    ]))
    .map_err(|_| invalid_data("parameter count overflow"))?;
    let custom_len = usize::try_from(u32::from_le_bytes([
        header[16], header[17], header[18], header[19],
    ]))
    .map_err(|_| invalid_data("state length overflow"))?;
    let persist_len = if encoded_len == HEADER_LEN {
        usize::try_from(u32::from_le_bytes([
            header[20], header[21], header[22], header[23],
        ]))
        .map_err(|_| invalid_data("persist length overflow"))?
    } else {
        0
    };
    let total_state = custom_len
        .checked_add(persist_len)
        .ok_or_else(|| invalid_data("state length overflow"))?;
    if name_len == 0
        || name_len > MAX_NAME_BYTES
        || param_count > MAX_PARAMS
        || total_state > MAX_CUSTOM_STATE_BYTES
    {
        return Err(invalid_data("preset field exceeds its bound"));
    }
    Ok(PresetHeader {
        encoded_len,
        name_len,
        param_count,
        custom_len,
        persist_len,
    })
}

fn preset_length(header: PresetHeader) -> io::Result<usize> {
    header
        .param_count
        .checked_mul(PARAM_LEN)
        .and_then(|params| {
            header
                .encoded_len
                .checked_add(header.name_len)?
                .checked_add(params)
        })
        .and_then(|length| length.checked_add(header.custom_len))
        .and_then(|length| length.checked_add(header.persist_len))
        .ok_or_else(|| invalid_data("preset length overflow"))
}

pub(super) fn validate_snapshot(
    params: &[(u32, f64)],
    custom_len: usize,
    persist_len: usize,
) -> io::Result<()> {
    let state_len = custom_len
        .checked_add(persist_len)
        .ok_or_else(|| invalid_input("preset snapshot exceeds its bound"))?;
    if params.len() > MAX_PARAMS || state_len > MAX_CUSTOM_STATE_BYTES {
        return Err(invalid_input("preset snapshot exceeds its bound"));
    }
    for (index, (id, normalized)) in params.iter().enumerate() {
        if !normalized.is_finite() || !(0.0..=1.0).contains(normalized) {
            return Err(invalid_data("invalid normalized parameter value"));
        }
        if params[..index].iter().any(|(previous, _)| previous == id) {
            return Err(invalid_data("duplicate parameter ID"));
        }
    }
    Ok(())
}
