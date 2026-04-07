//! WASM binary loader — validates the magic/version header and splits the
//! binary into named section byte-slices.
//!
//! No heap allocation is performed.  Every field of [`Module`] is an
//! `Option<&'a [u8]>` that points directly into the input buffer.

// ── Section IDs ─────────────────────────────────────────────────────────────
pub const SECTION_TYPE:     u8 = 1;
pub const SECTION_IMPORT:   u8 = 2;
pub const SECTION_FUNCTION: u8 = 3;
pub const SECTION_TABLE:    u8 = 4;
pub const SECTION_MEMORY:   u8 = 5;
pub const SECTION_GLOBAL:   u8 = 6;
pub const SECTION_EXPORT:   u8 = 7;
pub const SECTION_ELEMENT:  u8 = 9;
pub const SECTION_CODE:     u8 = 10;
pub const SECTION_DATA:     u8 = 11;

const MAGIC:   [u8; 4] = [0x00, 0x61, 0x73, 0x6D]; // "\0asm"
const VERSION: [u8; 4] = [0x01, 0x00, 0x00, 0x00];

// ── Error type ───────────────────────────────────────────────────────────────

/// Errors that can occur while parsing a WASM binary.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LoadError {
    /// Binary is shorter than the 8-byte header.
    TooShort,
    /// Magic bytes (`\0asm`) are missing or wrong.
    BadMagic,
    /// Version field is not `0x01 0x00 0x00 0x00`.
    BadVersion,
    /// A LEB-128 integer could not be decoded.
    InvalidLeb128,
    /// A section's declared byte-length runs past the end of the buffer.
    UnexpectedEof,
}

impl LoadError {
    /// Return a human-readable description of the error.
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

/// Zero-copy view of a WASM binary's sections.
///
/// Every field is a raw byte slice borrowed from the original `bytes` buffer
/// passed to [`load`].  Unknown sections (e.g. custom/debug) are silently
/// skipped; unused sections remain `None`.
pub struct Module<'a> {
    /// Type section (function signatures).
    pub type_section:     Option<&'a [u8]>,
    /// Import section (host functions / memories / tables / globals).
    pub import_section:   Option<&'a [u8]>,
    /// Function section (type-index per defined function).
    pub function_section: Option<&'a [u8]>,
    /// Table section (function-reference tables).
    pub table_section:    Option<&'a [u8]>,
    /// Memory section (linear memory limits).
    pub memory_section:   Option<&'a [u8]>,
    /// Global section (mutable/immutable globals with init expressions).
    pub global_section:   Option<&'a [u8]>,
    /// Export section (exported functions, memories, tables, globals).
    pub export_section:   Option<&'a [u8]>,
    /// Element section (function-table initializers).
    pub element_section:  Option<&'a [u8]>,
    /// Code section (function bodies).
    pub code_section:     Option<&'a [u8]>,
    /// Data section (linear-memory initializers).
    pub data_section:     Option<&'a [u8]>,
}

// ── LEB-128 helper ───────────────────────────────────────────────────────────

/// Decode an unsigned 32-bit LEB-128 integer from the front of `bytes`.
///
/// Returns `Some((value, bytes_consumed))` on success, or `None` if the
/// encoding is truncated or would overflow a `u32`.
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

/// Search the export section for a function export named `name`.
///
/// Returns the absolute function index (imports counted first) if a matching
/// function export is found, or `None` if the name is absent, the export is
/// not a function, or the section is malformed.
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

// ── Import section iterator ───────────────────────────────────────────────────

/// Iterate over every **function** import in the section, calling
/// `f(module_name, func_name)` for each one in declaration order.
///
/// Non-function imports (table, memory, global) are skipped silently.
/// Iteration stops early on any parse error.
pub fn for_each_func_import<F: FnMut(&str, &str)>(section: &[u8], f: &mut F) {
    fn inner<F: FnMut(&str, &str)>(section: &[u8], f: &mut F) -> Option<()> {
        let mut cur = 0usize;
        let (count, n) = read_u32_leb128(section)?;
        cur += n;

        for _ in 0..count as usize {
            // Module name
            let (ml, n) = read_u32_leb128(&section[cur..])?; cur += n;
            let mod_name = core::str::from_utf8(section.get(cur..cur + ml as usize)?).ok()?;
            cur += ml as usize;

            // Field name
            let (nl, n) = read_u32_leb128(&section[cur..])?; cur += n;
            let func_name = core::str::from_utf8(section.get(cur..cur + nl as usize)?).ok()?;
            cur += nl as usize;

            // Kind byte
            let kind = *section.get(cur)?; cur += 1;

            match kind {
                0 => { // function: type index
                    let (_, n) = read_u32_leb128(&section[cur..])?; cur += n;
                    f(mod_name, func_name);
                }
                1 => { // table: reftype + limits
                    cur += 1; // reftype
                    let flag = *section.get(cur)?; cur += 1;
                    let (_, n) = read_u32_leb128(&section[cur..])?; cur += n;
                    if flag != 0 { let (_, n) = read_u32_leb128(&section[cur..])?; cur += n; }
                }
                2 => { // memory: limits
                    let flag = *section.get(cur)?; cur += 1;
                    let (_, n) = read_u32_leb128(&section[cur..])?; cur += n;
                    if flag != 0 { let (_, n) = read_u32_leb128(&section[cur..])?; cur += n; }
                }
                3 => { cur += 2; } // global: valtype + mutability
                _ => return None,
            }
        }
        Some(())
    }
    inner(section, f);
}

// ── Memory section helper ─────────────────────────────────────────────────────

/// Parse the memory section and return the minimum page count declared by
/// the module.
///
/// Returns `0` if the section is absent or malformed.  The layout parsed is:
/// ```text
/// count : u32 LEB128   (MVP always 1)
/// flags : u8           (0 = no max, 1 = has max)
/// min   : u32 LEB128
/// [max  : u32 LEB128]  (only when flags == 1)
/// ```
pub fn read_memory_min_pages(section: &[u8]) -> u32 {
    fn inner(section: &[u8]) -> Option<u32> {
        let mut cur = 0usize;
        let (count, n) = read_u32_leb128(section)?;
        cur += n;
        if count == 0 { return Some(0); }
        if cur >= section.len() { return None; }
        cur += 1; // skip flags byte — we only need min
        let (min, _) = read_u32_leb128(&section[cur..])?;
        Some(min)
    }
    inner(section).unwrap_or(0)
}

// ── Main entry point ─────────────────────────────────────────────────────────

/// Parse a WASM binary buffer.
///
/// Validates the 8-byte header (`\0asm` magic + version `0x1`) and then
/// iterates over every section, storing a slice pointer for each known
/// section ID.  Unknown and custom sections (ID 0) are silently skipped.
///
/// On success, returns a [`Module`] whose fields borrow directly from
/// `bytes`.  No allocation is performed.
///
/// # Errors
///
/// Returns a [`LoadError`] if the header is invalid, a LEB-128 length field
/// is malformed, or a section length overruns the buffer.
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
        table_section:    None,
        memory_section:   None,
        global_section:   None,
        export_section:   None,
        element_section:  None,
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
            SECTION_TABLE    => module.table_section    = Some(data),
            SECTION_MEMORY   => module.memory_section   = Some(data),
            SECTION_GLOBAL   => module.global_section   = Some(data),
            SECTION_EXPORT   => module.export_section   = Some(data),
            SECTION_ELEMENT  => module.element_section  = Some(data),
            SECTION_CODE     => module.code_section     = Some(data),
            SECTION_DATA     => module.data_section     = Some(data),
            _                => {} // custom sections — skip
        }
    }

    Ok(module)
}
