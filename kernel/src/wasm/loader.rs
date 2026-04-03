/// WASM binary loader — validates the header and splits the binary into
/// named section byte-slices.  No heap allocation; all slices point into
/// the original `bytes` buffer.

// ── Section IDs ─────────────────────────────────────────────────────────────
pub const SECTION_TYPE:     u8 = 1;
pub const SECTION_IMPORT:   u8 = 2;
pub const SECTION_FUNCTION: u8 = 3;
pub const SECTION_GLOBAL:   u8 = 6;
pub const SECTION_EXPORT:   u8 = 7;
pub const SECTION_CODE:     u8 = 10;
pub const SECTION_DATA:     u8 = 11;

const MAGIC:   [u8; 4] = [0x00, 0x61, 0x73, 0x6D]; // "\0asm"
const VERSION: [u8; 4] = [0x01, 0x00, 0x00, 0x00];

// ── Error type ───────────────────────────────────────────────────────────────
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LoadError {
    TooShort,
    BadMagic,
    BadVersion,
    InvalidLeb128,
    UnexpectedEof,
}

impl LoadError {
    pub fn as_str(self) -> &'static str {
        match self {
            LoadError::TooShort      => "binary too short",
            LoadError::BadMagic      => "bad magic (not a WASM file)",
            LoadError::BadVersion    => "unsupported WASM version",
            LoadError::InvalidLeb128 => "malformed LEB-128 integer",
            LoadError::UnexpectedEof => "unexpected end of section data",
        }
    }
}

// ── Parsed module ────────────────────────────────────────────────────────────
/// Holds zero-copy slices into the original binary buffer.
/// Only the sections the interpreter actually needs are captured;
/// everything else is skipped over silently.
pub struct Module<'a> {
    pub type_section:     Option<&'a [u8]>,
    pub import_section:   Option<&'a [u8]>,
    pub function_section: Option<&'a [u8]>,
    pub global_section:   Option<&'a [u8]>,
    pub export_section:   Option<&'a [u8]>,
    pub code_section:     Option<&'a [u8]>,
    pub data_section:     Option<&'a [u8]>,
}

// ── LEB-128 helper ───────────────────────────────────────────────────────────
/// Decode an unsigned 32-bit LEB-128 integer from the front of `bytes`.
/// Returns `(value, bytes_consumed)` or `None` on malformed input.
pub fn read_u32_leb128(bytes: &[u8]) -> Option<(u32, usize)> {
    let mut result: u32 = 0;
    let mut shift: u32 = 0;
    for (i, &byte) in bytes.iter().enumerate() {
        if shift >= 35 {
            return None; // would overflow u32
        }
        result |= ((byte & 0x7F) as u32) << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            return Some((result, i + 1));
        }
    }
    None // ran out of bytes before the terminating byte
}

// ── Export section lookup ────────────────────────────────────────────────────

/// Search the export section for an export named `name` with kind = func (0).
/// Returns the absolute function index (imports included) if found.
pub fn find_export(module: &Module, name: &str) -> Option<u32> {
    let bytes = module.export_section?;
    let mut cur = 0usize;

    let (count, n) = read_u32_leb128(&bytes[cur..])?;
    cur += n;

    for _ in 0..count as usize {
        // Name
        let (name_len, n) = read_u32_leb128(&bytes[cur..])?;
        cur += n;
        let name_end = cur + name_len as usize;
        if name_end > bytes.len() { return None; }
        let export_name = core::str::from_utf8(&bytes[cur..name_end]).ok()?;
        cur = name_end;

        // Kind
        if cur >= bytes.len() { return None; }
        let kind = bytes[cur]; cur += 1;

        // Index
        let (index, n) = read_u32_leb128(&bytes[cur..])?;
        cur += n;

        if kind == 0 && export_name == name {
            return Some(index);
        }
    }
    None
}

// ── Main entry point ─────────────────────────────────────────────────────────
/// Parse `bytes` as a WASM binary.
/// On success returns a `Module` whose slices reference `bytes` directly.
pub fn load(bytes: &[u8]) -> Result<Module<'_>, LoadError> {
    // ── 1. Header ──
    if bytes.len() < 8 {
        return Err(LoadError::TooShort);
    }
    if bytes[..4] != MAGIC {
        return Err(LoadError::BadMagic);
    }
    if bytes[4..8] != VERSION {
        return Err(LoadError::BadVersion);
    }

    let mut module = Module {
        type_section:     None,
        import_section:   None,
        function_section: None,
        global_section:   None,
        export_section:   None,
        code_section:     None,
        data_section:     None,
    };

    // ── 2. Section loop ──
    let mut cursor = 8;
    while cursor < bytes.len() {
        let section_id = bytes[cursor];
        cursor += 1;

        // Section payload length (LEB-128)
        let (size, consumed) = read_u32_leb128(&bytes[cursor..])
            .ok_or(LoadError::InvalidLeb128)?;
        cursor += consumed;

        let end = cursor + size as usize;
        if end > bytes.len() {
            return Err(LoadError::UnexpectedEof);
        }
        let data = &bytes[cursor..end];
        cursor = end;

        match section_id {
            SECTION_TYPE     => module.type_section     = Some(data),
            SECTION_IMPORT   => module.import_section   = Some(data),
            SECTION_FUNCTION => module.function_section = Some(data),
            SECTION_GLOBAL   => module.global_section   = Some(data),
            SECTION_EXPORT   => module.export_section   = Some(data),
            SECTION_CODE     => module.code_section     = Some(data),
            SECTION_DATA     => module.data_section     = Some(data),
            _                => {} // custom / table / memory / etc. — skip
        }
    }

    Ok(module)
}
