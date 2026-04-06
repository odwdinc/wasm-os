use alloc::string::String;
use alloc::vec::Vec;

// ---------------- Tokenizer & Parsing ----------------
#[derive(Debug, Copy, Clone)]
pub enum Token<'a> {
    LParen,
    RParen,
    Keyword(&'a str),
    Identifier(&'a str),
    Int(i32),
    Float(f32),
    StringLiteral(&'a str),
}

pub struct Tokenizer<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Tokenizer<'a> {
    pub fn new(src: &'a str) -> Self { Self { src, pos: 0 } }

    pub fn next_token(&mut self) -> Option<Token<'a>> {
        self.skip_whitespace_and_comments();
        if self.pos >= self.src.len() { return None; }
        let c = self.src.as_bytes()[self.pos] as char;
        match c {
            '(' => { self.pos += 1; Some(Token::LParen) }
            ')' => { self.pos += 1; Some(Token::RParen) }
            '$' => Some(self.read_identifier()),
            '"' => Some(self.read_string()),
            '0'..='9' | '-' => Some(self.read_number()),
            _ => Some(self.read_keyword()),
        }
    }

    fn skip_whitespace_and_comments(&mut self) {
        while self.pos < self.src.len() {
            let c = self.src.as_bytes()[self.pos] as char;
            if c.is_whitespace() { self.pos += 1; }
            else if c == ';' && self.peek_next_char() == Some(';') {
                self.pos += 2;
                while self.pos < self.src.len() && self.src.as_bytes()[self.pos] as char != '\n' { self.pos += 1; }
            } else { break; }
        }
    }

    fn peek_next_char(&self) -> Option<char> {
        if self.pos + 1 < self.src.len() { Some(self.src.as_bytes()[self.pos + 1] as char) } else { None }
    }

    fn read_identifier(&mut self) -> Token<'a> {
        let start = self.pos;
        self.pos += 1; // skip '$'
        while self.pos < self.src.len() {
            let c = self.src.as_bytes()[self.pos] as char;
            if c.is_alphanumeric() || c == '_' || c == '-' || c == '.' { self.pos += 1; } else { break; }
        }
        Token::Identifier(&self.src[start..self.pos])
    }

    fn read_string(&mut self) -> Token<'a> {
        self.pos += 1; // skip opening "
        let start = self.pos;
        while self.pos < self.src.len() {
            let c = self.src.as_bytes()[self.pos] as char;
            if c == '"' { break; }
            self.pos += 1;
        }
        let s = &self.src[start..self.pos];
        if self.pos < self.src.len() { self.pos += 1; } // skip closing "
        Token::StringLiteral(s)
    }

    fn read_number(&mut self) -> Token<'a> {
        let start = self.pos;
        let mut is_float = false;
        if self.pos < self.src.len() && self.src.as_bytes()[self.pos] == b'-' {
            self.pos += 1;
        }
        while self.pos < self.src.len() {
            let c = self.src.as_bytes()[self.pos] as char;
            if c == '.' { is_float = true; self.pos += 1; }
            else if c.is_ascii_digit() { self.pos += 1; }
            else { break; }
        }
        let s = &self.src[start..self.pos];
        if is_float { Token::Float(parse_f32(s)) } else { Token::Int(parse_i32(s)) }
    }

    fn read_keyword(&mut self) -> Token<'a> {
        let start = self.pos;
        while self.pos < self.src.len() {
            let c = self.src.as_bytes()[self.pos] as char;
            if c.is_whitespace() || c == '(' || c == ')' { break; }
            self.pos += 1;
        }
        Token::Keyword(&self.src[start..self.pos])
    }
}

fn parse_i32(s: &str) -> i32 {
    let mut res = 0i32; let mut neg = false; let mut chars = s.chars();
    if let Some('-') = chars.clone().next() { neg = true; chars.next(); }
    for c in chars { if c.is_ascii_digit() { res = res * 10 + (c as i32 - '0' as i32); } }
    if neg { -res } else { res }
}

fn parse_f32(s: &str) -> f32 {
    let neg = s.starts_with('-');
    let mut parts = s.trim_start_matches('-').split('.');
    let int_part = parse_i32(parts.next().unwrap_or("0")) as f32;
    let frac = if let Some(frac_str) = parts.next() {
        let mut f = 0f32;
        let mut div = 10f32;
        for c in frac_str.chars() {
            f += (c as u8 - b'0') as f32 / div;
            div *= 10.0;
        }
        f
    } else { 0.0 };
    let result = int_part + frac;
    if neg { -result } else { result }
}

// ---------------- Value Types ----------------

/// WASM value type encoding byte.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum ValType {
    I32,  // 0x7F
    I64,  // 0x7E
    F32,  // 0x7D
    F64,  // 0x7C
}

impl ValType {
    pub fn byte(self) -> u8 {
        match self {
            ValType::I32 => 0x7F,
            ValType::I64 => 0x7E,
            ValType::F32 => 0x7D,
            ValType::F64 => 0x7C,
        }
    }

    /// Parse a WAT keyword like "i32", "i64", "f32", "f64".
    pub fn from_kw(kw: &str) -> Option<ValType> {
        match kw {
            "i32" => Some(ValType::I32),
            "i64" => Some(ValType::I64),
            "f32" => Some(ValType::F32),
            "f64" => Some(ValType::F64),
            _ => None,
        }
    }
}

// ---------------- IR types ----------------

/// A function type signature.
#[derive(Clone)]
pub struct FuncType {
    pub params:  Vec<ValType>,
    pub results: Vec<ValType>,
}

/// A local variable inside a function body.
#[derive(Clone)]
pub struct Local {
    pub ty: ValType,
}

/// A single WASM instruction (only the subset we parse from WAT).
#[derive(Clone)]
pub enum Instr {
    // Control
    Unreachable,
    Nop,
    Return,
    Drop,
    // Locals
    LocalGet(u32),
    LocalSet(u32),
    LocalTee(u32),
    // Constants
    I32Const(i32),
    I64Const(i32),   // store as i32 for now (LEB-encoded)
    F32Const(f32),
    F64Const(f32),   // store as f32 for now
    // i32 arithmetic
    I32Add, I32Sub, I32Mul, I32DivS, I32DivU,
    I32RemS, I32RemU,
    I32And, I32Or,  I32Xor,
    I32Shl, I32ShrS, I32ShrU,
    I32Eqz, I32Eq, I32Ne,
    I32LtS, I32LtU, I32GtS, I32GtU,
    I32LeS, I32LeU, I32GeS, I32GeU,
    // i64 arithmetic
    I64Add, I64Sub, I64Mul,
    I64And, I64Or,  I64Xor,
    I64Eqz, I64Eq, I64Ne,
    // f32/f64
    F32Add, F32Sub, F32Mul, F32Div,
    F64Add, F64Sub, F64Mul, F64Div,
    // Memory
    I32Load  { offset: u32, align: u32 },
    I32Store { offset: u32, align: u32 },
    I32Load8S  { offset: u32, align: u32 },
    I32Load8U  { offset: u32, align: u32 },
    I32Store8  { offset: u32, align: u32 },
    // Calls
    Call(u32),
    // Block / if / loop (simplified: no nesting stack — emit end directly)
    If { block_ty: u8 },   // 0x40 = void
    Else,
    Loop { block_ty: u8 },
    Block { block_ty: u8 },
    Br(u32),
    BrIf(u32),
    End,
    // Select
    Select,
}

/// A parsed function (may be local or imported, here: local only).
pub struct FuncDef {
    pub type_idx: u32,
    pub locals:   Vec<Local>,
    pub body:     Vec<Instr>,
}

/// (import "module" "name" (func (type $idx)))
pub struct Import {
    pub module:   Vec<u8>,   // string bytes
    pub name:     Vec<u8>,
    pub type_idx: u32,
}

/// (export "name" (func $idx))
pub struct Export {
    pub name:     Vec<u8>,
    pub kind:     u8,        // 0x00 = func
    pub index:    u32,
}

/// (memory min max?)
pub struct Memory {
    pub min: u32,
    pub max: Option<u32>,
}

/// (data (i32.const offset) "bytes...")
pub struct DataSeg {
    pub mem_idx: u32,
    pub offset:  i32,
    pub bytes:   Vec<u8>,
}

/// Complete module IR.
pub struct Module {
    pub types:    Vec<FuncType>,
    pub imports:  Vec<Import>,
    pub funcs:    Vec<FuncDef>,
    pub memories: Vec<Memory>,
    pub exports:  Vec<Export>,
    pub data:     Vec<DataSeg>,
}

// ---------------- Parser ----------------

pub struct Parser<'a> {
    tokenizer: Tokenizer<'a>,
    lookahead: Option<Token<'a>>,
    /// One-token unget slot: if Some, bump() puts lookahead here first.
    unget: Option<Token<'a>>,
    /// Map $name -> func index, built as we parse func definitions.
    func_names: Vec<(&'a str, u32)>,
    /// Map $name -> type index.
    type_names: Vec<(&'a str, u32)>,
    /// Map $name -> local index for the current function (params + locals).
    /// Reset at the start of each func definition.
    local_names: Vec<(&'a str, u32)>,
}

impl<'a> Parser<'a> {
    pub fn new(src: &'a str) -> Self {
        let mut t = Tokenizer::new(src);
        let la = t.next_token();
        Self { tokenizer: t, lookahead: la, unget: None, func_names: Vec::new(), type_names: Vec::new(), local_names: Vec::new() }
    }

    fn bump(&mut self) {
        if let Some(tok) = self.unget.take() {
            // Drain the unget slot: push current lookahead aside, restore unget tok.
            // Actually unget means: lookahead = unget, next real token comes after.
            // So: current lookahead becomes the NEW unget, unget becomes lookahead.
            // Wait -- unget_lparen sets unget=Some(LParen) after we already bumped.
            // So unget holds a token that should come BEFORE the current lookahead.
            // Correct drain: lookahead = tok (the unget), leave tokenizer alone.
            self.lookahead = Some(tok);
        } else {
            self.lookahead = self.tokenizer.next_token();
        }
    }

    /// Restore a consumed LParen back as the current lookahead,
    /// saving the current lookahead into the unget slot.
    fn unget_lparen(&mut self) {
        // Current lookahead goes into unget (to be read after the LParen).
        self.unget = self.lookahead.take();
        self.lookahead = Some(Token::LParen);
    }

    fn expect_lparen(&mut self) {
        match self.lookahead {
            Some(Token::LParen) => self.bump(),
            _ => panic!("expected '('"),
        }
    }

    fn expect_rparen(&mut self) {
        match self.lookahead {
            Some(Token::RParen) => self.bump(),
            _ => panic!("expected ')'"),
        }
    }

    fn expect_keyword(&mut self, kw: &str) {
        match self.lookahead {
            Some(Token::Keyword(k)) if k == kw => self.bump(),
            _ => panic!("expected keyword '{}'", kw),
        }
    }

    /// Consume a keyword and return it, or None if next token is not a keyword.
    fn try_keyword(&mut self) -> Option<&'a str> {
        match self.lookahead {
            Some(Token::Keyword(k)) => { self.bump(); Some(k) }
            _ => None,
        }
    }

    /// Consume an identifier ($name) and return the slice, or None.
    fn try_identifier(&mut self) -> Option<&'a str> {
        match self.lookahead {
            Some(Token::Identifier(id)) => { self.bump(); Some(id) }
            _ => None,
        }
    }

    /// Consume a string literal and return its bytes.
    fn expect_string(&mut self) -> Vec<u8> {
        match self.lookahead {
            Some(Token::StringLiteral(s)) => {
                let bytes = unescape_wat_string(s);
                self.bump();
                bytes
            }
            _ => panic!("expected string literal"),
        }
    }

    /// Skip a balanced paren list (we are INSIDE the '(' already).
    fn skip_rest_of_list(&mut self) {
        let mut depth = 1i32;
        while depth > 0 {
            match self.lookahead {
                Some(Token::LParen)  => { depth += 1; self.bump(); }
                Some(Token::RParen)  => { depth -= 1; self.bump(); }
                Some(_) => { self.bump(); }
                None    => panic!("unexpected EOF while skipping"),
            }
        }
    }

    /// Returns the index into `types` for this FuncType, inserting if absent.
    fn intern_type(types: &mut Vec<FuncType>, ft: FuncType) -> u32 {
        for (i, t) in types.iter().enumerate() {
            if t.params == ft.params && t.results == ft.results {
                return i as u32;
            }
        }
        let idx = types.len() as u32;
        types.push(ft);
        idx
    }

    // ---- Top-level ----

    pub fn parse_module(&mut self) -> Module {
        let mut module = Module {
            types:    Vec::new(),
            imports:  Vec::new(),
            funcs:    Vec::new(),
            memories: Vec::new(),
            exports:  Vec::new(),
            data:     Vec::new(),
        };

        self.expect_lparen();
        self.expect_keyword("module");
        // Optional module name
        self.try_identifier();

        while let Some(Token::LParen) = self.lookahead {
            self.expect_lparen();
            match self.lookahead {
                Some(Token::Keyword("type"))   => { self.parse_type_def(&mut module); }
                Some(Token::Keyword("import")) => { self.parse_import(&mut module); }
                Some(Token::Keyword("func"))   => { self.parse_func_def(&mut module); }
                Some(Token::Keyword("memory")) => { self.parse_memory(&mut module); }
                Some(Token::Keyword("export")) => { self.parse_export(&mut module); }
                Some(Token::Keyword("data"))   => { self.parse_data(&mut module); }
                Some(Token::Keyword("table"))  |
                Some(Token::Keyword("elem"))   |
                Some(Token::Keyword("global")) |
                Some(Token::Keyword("start"))  => {
                    // consume keyword, skip section
                    self.bump();
                    self.skip_rest_of_list();
                }
                _ => {
                    self.skip_rest_of_list();
                }
            }
        }
        self.expect_rparen(); // closing (module)
        module
    }

    // ---- (type $name (func (param ...) (result ...))) ----

    fn parse_type_def(&mut self, module: &mut Module) {
        self.bump(); // consume "type"
        let name = self.try_identifier();
        self.expect_lparen();
        self.expect_keyword("func");
        let ft = self.parse_func_type_fields();
        self.expect_rparen(); // close (func)
        let idx = module.types.len() as u32;
        module.types.push(ft);
        if let Some(n) = name { self.type_names.push((n, idx)); }
        self.expect_rparen(); // close (type)
    }

    /// Parse (param ...) (result ...) fields until ')' or a non-param/result token.
    /// Does NOT consume the surrounding parens of the `func` itself.
    fn parse_func_type_fields(&mut self) -> FuncType {
        let mut params  = Vec::new();
        let mut results = Vec::new();
        while let Some(Token::LParen) = self.lookahead {
            self.expect_lparen();
            match self.lookahead {
                Some(Token::Keyword("param")) => {
                    self.bump();
                    self.try_identifier(); // optional $name
                    while let Some(Token::Keyword(kw)) = self.lookahead {
                        if let Some(vt) = ValType::from_kw(kw) {
                            params.push(vt); self.bump();
                        } else { break; }
                    }
                }
                Some(Token::Keyword("result")) => {
                    self.bump();
                    while let Some(Token::Keyword(kw)) = self.lookahead {
                        if let Some(vt) = ValType::from_kw(kw) {
                            results.push(vt); self.bump();
                        } else { break; }
                    }
                }
                _ => {
                    // Not param/result – put the lparen back conceptually by
                    // skipping this unknown sub-section.
                    self.skip_rest_of_list();
                    continue;
                }
            }
            self.expect_rparen();
        }
        FuncType { params, results }
    }

    // ---- (import "mod" "name" (func ...)) ----

    fn parse_import(&mut self, module: &mut Module) {
        self.bump(); // consume "import"
        let mod_bytes  = self.expect_string();
        let name_bytes = self.expect_string();

        self.expect_lparen();
        match self.lookahead {
            Some(Token::Keyword("func")) => {
                self.bump();
                // Register $name -> func index BEFORE parsing type,
                // so call instructions can resolve it.
                let fidx = module.imports.len() as u32;
                if let Some(n) = self.try_identifier() {
                    self.func_names.push((n, fidx));
                }

                // (type $idx) or inline param/result
                let type_idx = self.parse_type_use(module);

                // Register this import as a func index
                let fidx = module.imports.len() as u32;
                // (imported funcs come before local funcs in the index space)
                // We store the $name mapping if any name was recorded above.
                module.imports.push(Import {
                    module:   mod_bytes,
                    name:     name_bytes,
                    type_idx,
                });
                let _ = fidx;
            }
            _ => { self.skip_rest_of_list(); return; }
        }
        self.expect_rparen(); // close (func)
        self.expect_rparen(); // close (import)
    }

    /// Like parse_type_use but the opening '(' has already been consumed.
    fn finish_type_use_after_lparen(&mut self, module: &mut Module) -> u32 {
        if let Some(Token::Keyword("type")) = self.lookahead {
            self.bump();
            let idx = match self.lookahead {
                Some(Token::Identifier(id)) => { let id = id; self.bump(); self.resolve_type_name(id, &module.types) }
                Some(Token::Int(n))         => { let n = n as u32; self.bump(); n }
                _ => panic!("expected type index"),
            };
            self.expect_rparen();
            let _ft = self.parse_func_type_fields();
            return idx;
        }
        let ft = self.finish_inline_func_type_after_lparen(module);
        Self::intern_type(&mut module.types, ft)
    }

    /// Parse a `(type $idx)` reference OR inline `(param ...) (result ...)`,
    /// intern into module.types, and return the type index.
    fn parse_type_use(&mut self, module: &mut Module) -> u32 {
        // Peek: is it (type ...) ?
        if let Some(Token::LParen) = self.lookahead {
            // peek further: is the keyword "type"?
            self.expect_lparen();
            if let Some(Token::Keyword("type")) = self.lookahead {
                self.bump(); // consume "type"
                let idx = match self.lookahead {
                    Some(Token::Identifier(id)) => {
                        let id = id;
                        self.bump();
                        self.resolve_type_name(id, &module.types)
                    }
                    Some(Token::Int(n)) => { let n = n as u32; self.bump(); n }
                    _ => panic!("expected type index"),
                };
                self.expect_rparen(); // close (type)
                // There may also be inline param/result after; parse and ignore
                // (the type annotation already gave us the index).
                let _ft = self.parse_func_type_fields();
                return idx;
            } else {
                // It's an inline param/result; we already consumed the '('.
                // Re-parse from where we are (we consumed one LParen already).
                // Handle the current keyword.
                let ft = self.finish_inline_func_type_after_lparen(module);
                return Self::intern_type(&mut module.types, ft);
            }
        }
        // No '(' at all – empty signature.
        let ft = FuncType { params: Vec::new(), results: Vec::new() };
        Self::intern_type(&mut module.types, ft)
    }

    /// We have consumed the '(' of the first param/result group; finish parsing.
    fn finish_inline_func_type_after_lparen(&mut self, _module: &mut Module) -> FuncType {
        let mut params  = Vec::new();
        let mut results = Vec::new();

        // Handle the first group (already consumed its '(')
        self.parse_param_or_result_group(&mut params, &mut results);

        // Handle subsequent groups
        while let Some(Token::LParen) = self.lookahead {
            self.expect_lparen();
            self.parse_param_or_result_group(&mut params, &mut results);
        }
        FuncType { params, results }
    }

    fn parse_param_or_result_group(&mut self, params: &mut Vec<ValType>, results: &mut Vec<ValType>) {
        match self.lookahead {
            Some(Token::Keyword("param")) => {
                self.bump();
                self.try_identifier();
                while let Some(Token::Keyword(kw)) = self.lookahead {
                    if let Some(vt) = ValType::from_kw(kw) { params.push(vt); self.bump(); }
                    else { break; }
                }
            }
            Some(Token::Keyword("result")) => {
                self.bump();
                while let Some(Token::Keyword(kw)) = self.lookahead {
                    if let Some(vt) = ValType::from_kw(kw) { results.push(vt); self.bump(); }
                    else { break; }
                }
            }
            _ => { self.skip_rest_of_list(); return; }
        }
        self.expect_rparen();
    }

    fn resolve_type_name(&self, id: &str, _types: &[FuncType]) -> u32 {
        for (n, idx) in &self.type_names {
            if *n == id { return *idx; }
        }
        // Try numeric after '$'
        let digits = id.trim_start_matches('$');
        parse_i32(digits) as u32
    }

    // ---- (func $name ...) ----

    fn parse_func_def(&mut self, module: &mut Module) {
        self.bump(); // consume "func"
        let name = self.try_identifier();

        // total func index = imports + local funcs so far
        let fidx = (module.imports.len() + module.funcs.len()) as u32;
        if let Some(n) = name { self.func_names.push((n, fidx)); }

        // Reset local name table for this function.
        self.local_names.clear();
        // next_local tracks the index to assign to the next named param/local.
        let mut next_local: u32 = 0;

        // A func body has this grammar (all optional, in order):
        //   (export "name")* (import "m" "n")?
        //   (type $t)? (param ...)* (result ...)*
        //   (local ...)*
        //   instr*
        // We peel off '(' groups one at a time and dispatch by keyword.

        let mut type_parsed   = false;
        let mut type_idx      = 0u32;
        let mut params: Vec<ValType>  = Vec::new();
        let mut results: Vec<ValType> = Vec::new();
        let mut locals: Vec<Local>    = Vec::new();

        loop {
            match self.lookahead {
                Some(Token::LParen) => {
                    self.bump(); // consume '('
                    match self.lookahead {
                        Some(Token::Keyword("export")) => {
                            self.bump();
                            let export_name = self.expect_string();
                            self.expect_rparen();
                            module.exports.push(Export { name: export_name, kind: 0x00, index: fidx });
                        }
                        Some(Token::Keyword("import")) => {
                            self.bump();
                            let mod_bytes  = self.expect_string();
                            let name_bytes = self.expect_string();
                            self.expect_rparen(); // close (import)
                            // Finish parsing the type inline, then register as import.
                            let ft = FuncType { params, results };
                            let ti = Self::intern_type(&mut module.types, ft);
                            module.imports.push(Import { module: mod_bytes, name: name_bytes, type_idx: ti });
                            self.expect_rparen(); // close (func)
                            return;
                        }
                        Some(Token::Keyword("type")) => {
                            self.bump();
                            type_idx = match self.lookahead {
                                Some(Token::Identifier(id)) => { let id = id; self.bump(); self.resolve_type_name(id, &module.types) }
                                Some(Token::Int(n))         => { let n = n as u32; self.bump(); n }
                                _ => panic!("expected type index"),
                            };
                            type_parsed = true;
                            self.expect_rparen();
                        }
                        Some(Token::Keyword("param")) => {
                            self.bump();
                            // Optional $name — if present, this param gets that name at next_local
                            if let Some(param_name) = self.try_identifier() {
                                // Named param: exactly one type follows
                                if let Some(Token::Keyword(kw)) = self.lookahead {
                                    if let Some(vt) = ValType::from_kw(kw) {
                                        self.local_names.push((param_name, next_local));
                                        next_local += 1;
                                        params.push(vt);
                                        self.bump();
                                    }
                                }
                            } else {
                                // Unnamed: consume all types
                                while let Some(Token::Keyword(kw)) = self.lookahead {
                                    if let Some(vt) = ValType::from_kw(kw) {
                                        next_local += 1;
                                        params.push(vt);
                                        self.bump();
                                    } else { break; }
                                }
                            }
                            self.expect_rparen();
                        }
                        Some(Token::Keyword("result")) => {
                            self.bump();
                            while let Some(Token::Keyword(kw)) = self.lookahead {
                                if let Some(vt) = ValType::from_kw(kw) { results.push(vt); self.bump(); }
                                else { break; }
                            }
                            self.expect_rparen();
                        }
                        Some(Token::Keyword("local")) => {
                            // Intern the type before we start locals (params/results done).
                            if !type_parsed {
                                let ft = FuncType { params: params.clone(), results: results.clone() };
                                type_idx = Self::intern_type(&mut module.types, ft);
                                type_parsed = true;
                            }
                            self.bump();
                            // Optional $name
                            if let Some(local_name) = self.try_identifier() {
                                // Named local: exactly one type
                                if let Some(Token::Keyword(kw)) = self.lookahead {
                                    if let Some(vt) = ValType::from_kw(kw) {
                                        self.local_names.push((local_name, next_local));
                                        next_local += 1;
                                        locals.push(Local { ty: vt });
                                        self.bump();
                                    }
                                }
                            } else {
                                while let Some(Token::Keyword(kw)) = self.lookahead {
                                    if let Some(vt) = ValType::from_kw(kw) {
                                        next_local += 1;
                                        locals.push(Local { ty: vt });
                                        self.bump();
                                    } else { break; }
                                }
                            }
                            self.expect_rparen();
                        }
                        _ => {
                            // No more header groups — this '(' starts the body.
                            if !type_parsed {
                                let ft = FuncType { params, results };
                                type_idx = Self::intern_type(&mut module.types, ft);
                            }
                            // We already consumed the '(', so parse as folded instr.
                            let mut body = self.parse_folded_instr(module);
                            body.extend(self.parse_instrs(module));
                            module.funcs.push(FuncDef { type_idx, locals, body });
                            self.expect_rparen(); // close (func)
                            return;
                        }
                    }
                }
                Some(Token::RParen) | None => {
                    // Empty body.
                    break;
                }
                _ => {
                    // Flat instruction — body starts here.
                    break;
                }
            }
        }

        // Intern type if not yet done (no locals or body triggered it above).
        if !type_parsed {
            let ft = FuncType { params, results };
            type_idx = Self::intern_type(&mut module.types, ft);
        }

        let body = self.parse_instrs(module);
        module.funcs.push(FuncDef { type_idx, locals, body });
        self.expect_rparen(); // close (func)
    }

    // ---- Instruction parsing ----

    /// Parse a sequence of instructions until we hit ')' or EOF.
    fn parse_instrs(&mut self, module: &mut Module) -> Vec<Instr> {
        let mut instrs = Vec::new();
        loop {
            match self.lookahead {
                Some(Token::RParen) | None => break,
                Some(Token::LParen) => {
                    self.bump(); // consume '('
                    let mut folded = self.parse_folded_instr(module);
                    instrs.append(&mut folded);
                }
                Some(Token::Keyword(_)) => {
                    if let Some(instr) = self.parse_plain_instr(module) {
                        instrs.push(instr);
                    }
                }
                _ => { self.bump(); } // skip unexpected tokens
            }
        }
        instrs
    }

    /// We have already consumed the '('. Parse a folded s-expr instruction.
    /// Returns a flat list (args first, then the op).
    fn parse_folded_instr(&mut self, module: &mut Module) -> Vec<Instr> {
        let mut out = Vec::new();
        // Special: (if ...) (block ...) (loop ...)
        match self.lookahead {
            Some(Token::Keyword("if")) => {
                return self.parse_folded_if(module);
            }
            Some(Token::Keyword("block")) | Some(Token::Keyword("loop")) => {
                return self.parse_folded_block_or_loop(module);
            }
            _ => {}
        }

        // Generic folded: (op args* sub-exprs*)
        // Collect all sub-expressions first (operands), then the operator.
        // But we need the keyword first.
        let kw = match self.lookahead {
            Some(Token::Keyword(k)) => { let k = k; self.bump(); k }
            _ => { self.skip_rest_of_list(); return out; }
        };

        // Parse token-level immediates (index, constant value, etc.) and
        // build the final instruction — but do NOT push it yet.
        let op = self.build_instr(kw, module);

        // Now parse nested sub-expressions (the stack operands in folded form).
        // These must be emitted BEFORE the operator.
        let mut sub_instrs: Vec<Instr> = Vec::new();
        loop {
            match self.lookahead {
                Some(Token::LParen) => {
                    self.bump();
                    let mut si = self.parse_folded_instr(module);
                    sub_instrs.append(&mut si);
                }
                Some(Token::RParen) | None => break,
                _ => break,
            }
        }
        self.expect_rparen(); // close ')'

        // Correct order: operands first, then the operator.
        out.append(&mut sub_instrs);
        if let Some(instr) = op { out.push(instr); }
        out
    }

    /// Parse token-level immediates for `kw` and return the completed instruction.
    /// Returns None for unknown opcodes.
    fn build_instr(&mut self, kw: &str, module: &mut Module) -> Option<Instr> {
        let instr = match kw {
            "i32.const" => {
                let n = match self.lookahead {
                    Some(Token::Int(n)) => { let n = n; self.bump(); n }
                    _ => 0,
                };
                Instr::I32Const(n)
            }
            "i64.const" => {
                let n = match self.lookahead {
                    Some(Token::Int(n)) => { let n = n; self.bump(); n }
                    _ => 0,
                };
                Instr::I64Const(n)
            }
            "f32.const" => {
                let v = match self.lookahead {
                    Some(Token::Float(f)) => { let f = f; self.bump(); f }
                    Some(Token::Int(n))   => { let n = n; self.bump(); n as f32 }
                    _ => 0.0,
                };
                Instr::F32Const(v)
            }
            "f64.const" => {
                let v = match self.lookahead {
                    Some(Token::Float(f)) => { let f = f; self.bump(); f }
                    Some(Token::Int(n))   => { let n = n; self.bump(); n as f32 }
                    _ => 0.0,
                };
                Instr::F64Const(v)
            }
            "local.get" | "get_local" => {
                let idx = self.parse_local_or_func_idx();
                Instr::LocalGet(idx)
            }
            "local.set" | "set_local" => {
                let idx = self.parse_local_or_func_idx();
                Instr::LocalSet(idx)
            }
            "local.tee" | "tee_local" => {
                let idx = self.parse_local_or_func_idx();
                Instr::LocalTee(idx)
            }
            "call" => {
                let idx = self.parse_func_idx(module);
                Instr::Call(idx)
            }
            "br"    => { let l = self.parse_label_idx(); Instr::Br(l) }
            "br_if" => { let l = self.parse_label_idx(); Instr::BrIf(l) }
            "i32.load"   => { let (o,a) = self.parse_memarg(); Instr::I32Load   { offset: o, align: a } }
            "i32.store"  => { let (o,a) = self.parse_memarg(); Instr::I32Store  { offset: o, align: a } }
            "i32.load8_s"=> { let (o,a) = self.parse_memarg(); Instr::I32Load8S { offset: o, align: a } }
            "i32.load8_u"=> { let (o,a) = self.parse_memarg(); Instr::I32Load8U { offset: o, align: a } }
            "i32.store8" => { let (o,a) = self.parse_memarg(); Instr::I32Store8 { offset: o, align: a } }
            "if"    => { let bt = self.parse_block_type(); Instr::If   { block_ty: bt } }
            "else"  => Instr::Else,
            "end"   => Instr::End,
            "loop"  => { let bt = self.parse_block_type(); Instr::Loop { block_ty: bt } }
            "block" => { let bt = self.parse_block_type(); Instr::Block{ block_ty: bt } }
            "return"      => Instr::Return,
            "drop"        => Instr::Drop,
            "select"      => Instr::Select,
            "unreachable" => Instr::Unreachable,
            "nop"         => Instr::Nop,
            "i32.add"  => Instr::I32Add,  "i32.sub"   => Instr::I32Sub,
            "i32.mul"  => Instr::I32Mul,  "i32.div_s" => Instr::I32DivS,
            "i32.div_u"=> Instr::I32DivU, "i32.rem_s" => Instr::I32RemS,
            "i32.rem_u"=> Instr::I32RemU,
            "i32.and"  => Instr::I32And,  "i32.or"    => Instr::I32Or,
            "i32.xor"  => Instr::I32Xor,
            "i32.shl"  => Instr::I32Shl,  "i32.shr_s" => Instr::I32ShrS,
            "i32.shr_u"=> Instr::I32ShrU,
            "i32.eqz"  => Instr::I32Eqz,  "i32.eq"    => Instr::I32Eq,
            "i32.ne"   => Instr::I32Ne,
            "i32.lt_s" => Instr::I32LtS,  "i32.lt_u"  => Instr::I32LtU,
            "i32.gt_s" => Instr::I32GtS,  "i32.gt_u"  => Instr::I32GtU,
            "i32.le_s" => Instr::I32LeS,  "i32.le_u"  => Instr::I32LeU,
            "i32.ge_s" => Instr::I32GeS,  "i32.ge_u"  => Instr::I32GeU,
            "i64.add"  => Instr::I64Add,  "i64.sub"   => Instr::I64Sub,
            "i64.mul"  => Instr::I64Mul,
            "i64.and"  => Instr::I64And,  "i64.or"    => Instr::I64Or,
            "i64.xor"  => Instr::I64Xor,
            "i64.eqz"  => Instr::I64Eqz,  "i64.eq"    => Instr::I64Eq,
            "i64.ne"   => Instr::I64Ne,
            "f32.add"  => Instr::F32Add,  "f32.sub"   => Instr::F32Sub,
            "f32.mul"  => Instr::F32Mul,  "f32.div"   => Instr::F32Div,
            "f64.add"  => Instr::F64Add,  "f64.sub"   => Instr::F64Sub,
            "f64.mul"  => Instr::F64Mul,  "f64.div"   => Instr::F64Div,
            _ => { return None; } // unknown op – emit nothing
        };
        Some(instr)
    }

    fn parse_plain_instr(&mut self, module: &mut Module) -> Option<Instr> {
        let kw = match self.lookahead {
            Some(Token::Keyword(k)) => { let k = k; self.bump(); k }
            _ => return None,
        };
        self.build_instr(kw, module)
    }

    fn parse_folded_if(&mut self, module: &mut Module) -> Vec<Instr> {
        self.bump(); // consume "if"
        self.try_identifier(); // optional label
        let bt = self.parse_block_type();

        // Optional condition sub-expr
        let mut cond: Vec<Instr> = Vec::new();
        // Collect (then ...) and (else ...) and regular instrs
        let mut then_instrs: Vec<Instr> = Vec::new();
        let mut else_instrs: Vec<Instr> = Vec::new();

        // First parse any pre-then sub-expressions (the condition in folded form)
        loop {
            match self.lookahead {
                Some(Token::LParen) => {
                    // Peek for "then"
                    self.bump();
                    match self.lookahead {
                        Some(Token::Keyword("then")) => {
                            self.bump();
                            then_instrs = self.parse_instrs(module);
                            self.expect_rparen();
                            break;
                        }
                        _ => {
                            let mut sub = self.parse_folded_instr(module);
                            cond.append(&mut sub);
                        }
                    }
                }
                Some(Token::RParen) => break,
                _ => break,
            }
        }

        // Check for (else ...)
        if let Some(Token::LParen) = self.lookahead {
            self.bump();
            if let Some(Token::Keyword("else")) = self.lookahead {
                self.bump();
                else_instrs = self.parse_instrs(module);
                self.expect_rparen();
            } else {
                self.skip_rest_of_list();
            }
        }

        self.expect_rparen(); // close (if)

        let mut out = cond;
        out.push(Instr::If { block_ty: bt });
        out.append(&mut then_instrs);
        if !else_instrs.is_empty() {
            out.push(Instr::Else);
            out.append(&mut else_instrs);
        }
        out.push(Instr::End);
        out
    }

    fn parse_folded_block_or_loop(&mut self, module: &mut Module) -> Vec<Instr> {
        let is_loop = matches!(self.lookahead, Some(Token::Keyword("loop")));
        self.bump(); // consume "block" or "loop"
        self.try_identifier(); // optional label
        let bt = self.parse_block_type();
        let body = self.parse_instrs(module);
        self.expect_rparen();

        let mut out = Vec::new();
        if is_loop {
            out.push(Instr::Loop  { block_ty: bt });
        } else {
            out.push(Instr::Block { block_ty: bt });
        }
        out.extend(body);
        out.push(Instr::End);
        out
    }

    fn parse_block_type(&mut self) -> u8 {
        // (result i32)? → 0x7F, else 0x40 (void)
        // Only consume a '(result T)' block type annotation.
        // If the next '(' is anything else (e.g. condition expression),
        // leave it completely untouched.
        if let Some(Token::LParen) = self.lookahead {
            self.bump(); // consume '(' speculatively
            if let Some(Token::Keyword("result")) = self.lookahead {
                self.bump(); // consume "result"
                let bt = match self.lookahead {
                    Some(Token::Keyword(kw)) => {
                        ValType::from_kw(kw).map(|v| v.byte()).unwrap_or(0x40)
                    }
                    _ => 0x40,
                };
                self.bump(); // consume type keyword
                self.expect_rparen(); // close (result)
                return bt;
            } else {
                // Not a (result) — restore the '(' we just consumed.
                self.unget_lparen();
            }
        }
        0x40 // void
    }

    fn parse_memarg(&mut self) -> (u32, u32) {
        let mut offset = 0u32;
        let mut align  = 0u32;
        // Parse optional offset=N align=N
        loop {
            match self.lookahead {
                Some(Token::Keyword(kw)) if kw.starts_with("offset=") => {
                    let n = parse_i32(&kw[7..]) as u32;
                    offset = n;
                    self.bump();
                }
                Some(Token::Keyword(kw)) if kw.starts_with("align=") => {
                    let n = parse_i32(&kw[6..]) as u32;
                    align = n;
                    self.bump();
                }
                _ => break,
            }
        }
        (offset, align)
    }

    fn parse_label_idx(&mut self) -> u32 {
        match self.lookahead {
            Some(Token::Int(n))         => { let n = n as u32; self.bump(); n }
            Some(Token::Identifier(_))  => { self.bump(); 0 } // simplified
            _ => 0,
        }
    }

    fn parse_local_or_func_idx(&mut self) -> u32 {
        match self.lookahead {
            Some(Token::Int(n)) => { let n = n as u32; self.bump(); n }
            Some(Token::Identifier(id)) => {
                let id = id;
                self.bump();
                for (name, idx) in &self.local_names {
                    if *name == id { return *idx; }
                }
                // Fall back to numeric suffix after '$'
                parse_i32(id.trim_start_matches('$')) as u32
            }
            _ => 0,
        }
    }

    fn parse_func_idx(&mut self, _module: &Module) -> u32 {
        match self.lookahead {
            Some(Token::Int(n))           => { let n = n as u32; self.bump(); n }
            Some(Token::Identifier(id)) => {
                let id = id;
                self.bump();
                // Look up in func_names
                for (n, idx) in &self.func_names {
                    if *n == id { return *idx; }
                }
                0
            }
            _ => 0,
        }
    }

    // ---- (memory min max?) ----

    fn parse_memory(&mut self, module: &mut Module) {
        self.bump(); // consume "memory"
        self.try_identifier();
        // Check for (import "mod" "name") inline — skip for now
        if let Some(Token::LParen) = self.lookahead {
            self.bump();
            if let Some(Token::Keyword("import")) = self.lookahead {
                self.skip_rest_of_list();
                self.expect_rparen(); // close (memory)
                return;
            } else {
                self.skip_rest_of_list();
            }
        }
        let min = match self.lookahead {
            Some(Token::Int(n)) => { let n = n as u32; self.bump(); n }
            _ => 0,
        };
        let max = match self.lookahead {
            Some(Token::Int(n)) => { let n = n as u32; self.bump(); Some(n) }
            _ => None,
        };
        module.memories.push(Memory { min, max });
        self.expect_rparen(); // close (memory)
    }

    // ---- (export "name" (func $idx)) ----

    fn parse_export(&mut self, module: &mut Module) {
        self.bump(); // consume "export"
        let name_bytes = self.expect_string();
        self.expect_lparen();
        let kind = match self.lookahead {
            Some(Token::Keyword("func"))   => { self.bump(); 0x00 }
            Some(Token::Keyword("memory")) => { self.bump(); 0x02 }
            Some(Token::Keyword("table"))  => { self.bump(); 0x01 }
            Some(Token::Keyword("global")) => { self.bump(); 0x03 }
            _ => { self.skip_rest_of_list(); self.expect_rparen(); return; }
        };
        let index = match self.lookahead {
            Some(Token::Int(n))         => { let n = n as u32; self.bump(); n }
            Some(Token::Identifier(id)) => {
                let id = id; self.bump();
                let mut found = 0u32;
                for (n, idx) in &self.func_names {
                    if *n == id { found = *idx; break; }
                }
                found
            }
            _ => 0,
        };
        self.expect_rparen(); // close (func/memory/…)
        self.expect_rparen(); // close (export)
        module.exports.push(Export { name: name_bytes, kind, index });
    }

    // ---- (data (i32.const N) "bytes") ----

    fn parse_data(&mut self, module: &mut Module) {
        self.bump(); // consume "data"
        // Optional memory index
        let mem_idx = match self.lookahead {
            Some(Token::Int(n)) => { let n = n as u32; self.bump(); n }
            _ => 0,
        };
        // Offset expression: (i32.const N) or (offset (i32.const N))
        let offset = if let Some(Token::LParen) = self.lookahead {
            self.bump();
            // Could be "offset" wrapper
            if let Some(Token::Keyword("offset")) = self.lookahead {
                self.bump();
                self.bump(); // skip '(' of inner i32.const
            }
            // Expect i32.const
            self.expect_keyword("i32.const");
            let n = match self.lookahead {
                Some(Token::Int(n)) => { let n = n; self.bump(); n }
                _ => 0,
            };
            self.expect_rparen(); // close i32.const or offset
            n
        } else { 0 };

        // String data
        let mut bytes: Vec<u8> = Vec::new();
        while let Some(Token::StringLiteral(_)) = self.lookahead {
            let mut b = self.expect_string();
            bytes.append(&mut b);
        }

        module.data.push(DataSeg { mem_idx, offset, bytes });
        self.expect_rparen(); // close (data)
    }
}

// ---- String un-escaping ----

fn unescape_wat_string(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'n'  => { out.push(b'\n'); i += 2; }
                b't'  => { out.push(b'\t'); i += 2; }
                b'r'  => { out.push(b'\r'); i += 2; }
                b'"'  => { out.push(b'"');  i += 2; }
                b'\\' => { out.push(b'\\'); i += 2; }
                b'0'  => { out.push(0);     i += 2; }
                h1 if h1.is_ascii_hexdigit() && i + 2 < bytes.len() && bytes[i+2].is_ascii_hexdigit() => {
                    let hi = hex_digit(h1);
                    let lo = hex_digit(bytes[i + 2]);
                    out.push((hi << 4) | lo);
                    i += 3;
                }
                _ => { out.push(bytes[i]); i += 1; }
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

fn hex_digit(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0,
    }
}

// ---------------- WASM Emitter ----------------

pub struct WasmEmitter {
    buf: Vec<u8>,
}

impl WasmEmitter {
    pub fn new() -> Self { Self { buf: Vec::new() } }

    pub fn write_u8(&mut self, val: u8) { self.buf.push(val); }

    pub fn write_u32_raw(&mut self, val: u32) {
        self.buf.extend_from_slice(&val.to_le_bytes());
    }

    pub fn write_leb128_u32(&mut self, mut val: u32) {
        loop {
            let byte = (val & 0x7F) as u8;
            val >>= 7;
            if val == 0 { self.write_u8(byte); break; }
            else        { self.write_u8(byte | 0x80); }
        }
    }

    pub fn write_leb128_i32(&mut self, mut val: i32) {
        loop {
            let byte = (val & 0x7F) as u8;
            val >>= 6; // arithmetic shift
            let more = val != 0 && val != -1;
            if !more { self.write_u8(byte & 0x7F); break; }
            else     { self.write_u8(byte | 0x80); val >>= 1; }
        }
    }

    /// Write a length-prefixed byte vector.
    fn write_bytes(&mut self, bytes: &[u8]) {
        self.write_leb128_u32(bytes.len() as u32);
        self.buf.extend_from_slice(bytes);
    }

    /// Emit a section: write id, then length-prefixed contents.
    fn emit_section<F>(&mut self, id: u8, f: F)
    where F: FnOnce(&mut WasmEmitter)
    {
        let mut inner = WasmEmitter::new();
        f(&mut inner);
        self.write_u8(id);
        self.write_leb128_u32(inner.buf.len() as u32);
        self.buf.extend_from_slice(&inner.buf);
    }

    // ---- Full module emission ----

    pub fn emit_module(&mut self, module: &Module) {
        // Magic + version
        self.buf.extend_from_slice(&[0x00, 0x61, 0x73, 0x6D]);
        self.buf.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);

        if !module.types.is_empty() {
            self.emit_type_section(module);
        }
        if !module.imports.is_empty() {
            self.emit_import_section(module);
        }
        if !module.funcs.is_empty() {
            self.emit_function_section(module);
        }
        if !module.memories.is_empty() {
            self.emit_memory_section(module);
        }
        if !module.exports.is_empty() {
            self.emit_export_section(module);
        }
        if !module.data.is_empty() && !module.memories.is_empty() {
            self.emit_data_count_section(module); // required before code in WASM 2.0 if data exists
        }
        if !module.funcs.is_empty() {
            self.emit_code_section(module);
        }
        if !module.data.is_empty() {
            self.emit_data_section(module);
        }
    }

    // Section 1: Type
    fn emit_type_section(&mut self, module: &Module) {
        self.emit_section(0x01, |s| {
            s.write_leb128_u32(module.types.len() as u32);
            for ft in &module.types {
                s.write_u8(0x60); // func type
                s.write_leb128_u32(ft.params.len() as u32);
                for &vt in &ft.params  { s.write_u8(vt.byte()); }
                s.write_leb128_u32(ft.results.len() as u32);
                for &vt in &ft.results { s.write_u8(vt.byte()); }
            }
        });
    }

    // Section 2: Import
    fn emit_import_section(&mut self, module: &Module) {
        self.emit_section(0x02, |s| {
            s.write_leb128_u32(module.imports.len() as u32);
            for imp in &module.imports {
                s.write_bytes(&imp.module);
                s.write_bytes(&imp.name);
                s.write_u8(0x00); // func import
                s.write_leb128_u32(imp.type_idx);
            }
        });
    }

    // Section 3: Function (type indices for local funcs)
    fn emit_function_section(&mut self, module: &Module) {
        self.emit_section(0x03, |s| {
            s.write_leb128_u32(module.funcs.len() as u32);
            for f in &module.funcs {
                s.write_leb128_u32(f.type_idx);
            }
        });
    }

    // Section 5: Memory
    fn emit_memory_section(&mut self, module: &Module) {
        self.emit_section(0x05, |s| {
            s.write_leb128_u32(module.memories.len() as u32);
            for mem in &module.memories {
                if let Some(max) = mem.max {
                    s.write_u8(0x01); // has max
                    s.write_leb128_u32(mem.min);
                    s.write_leb128_u32(max);
                } else {
                    s.write_u8(0x00); // no max
                    s.write_leb128_u32(mem.min);
                }
            }
        });
    }

    // Section 7: Export
    fn emit_export_section(&mut self, module: &Module) {
        self.emit_section(0x07, |s| {
            s.write_leb128_u32(module.exports.len() as u32);
            for exp in &module.exports {
                s.write_bytes(&exp.name);
                s.write_u8(exp.kind);
                s.write_leb128_u32(exp.index);
            }
        });
    }

    // Section 12: Data count (needed when data section present, WASM 2.0)
    fn emit_data_count_section(&mut self, module: &Module) {
        self.emit_section(0x0C, |s| {
            s.write_leb128_u32(module.data.len() as u32);
        });
    }

    // Section 10: Code
    fn emit_code_section(&mut self, module: &Module) {
        self.emit_section(0x0A, |s| {
            s.write_leb128_u32(module.funcs.len() as u32);
            for func in &module.funcs {
                let mut body = WasmEmitter::new();

                // Locals (run-length encoded by type)
                let local_groups = compress_locals(&func.locals);
                body.write_leb128_u32(local_groups.len() as u32);
                for (count, ty) in &local_groups {
                    body.write_leb128_u32(*count);
                    body.write_u8(ty.byte());
                }

                // Instructions
                for instr in &func.body {
                    body.emit_instr(instr);
                }
                body.write_u8(0x0B); // end

                // Write length-prefixed body
                s.write_leb128_u32(body.buf.len() as u32);
                s.buf.extend_from_slice(&body.buf);
            }
        });
    }

    // Section 11: Data
    fn emit_data_section(&mut self, module: &Module) {
        self.emit_section(0x0B, |s| {
            s.write_leb128_u32(module.data.len() as u32);
            for seg in &module.data {
                s.write_u8(0x00); // active segment, memory index 0
                // i32.const offset; end
                s.write_u8(0x41); // i32.const
                s.write_leb128_i32(seg.offset);
                s.write_u8(0x0B); // end
                s.write_bytes(&seg.bytes);
            }
        });
    }

    // ---- Instruction emission ----

    fn emit_instr(&mut self, instr: &Instr) {
        match instr {
            Instr::Unreachable => self.write_u8(0x00),
            Instr::Nop        => self.write_u8(0x01),
            Instr::Return     => self.write_u8(0x0F),
            Instr::Drop       => self.write_u8(0x1A),
            Instr::Select     => self.write_u8(0x1B),
            Instr::End        => self.write_u8(0x0B),
            Instr::Else       => self.write_u8(0x05),

            Instr::Block { block_ty } => { self.write_u8(0x02); self.write_u8(*block_ty); }
            Instr::Loop  { block_ty } => { self.write_u8(0x03); self.write_u8(*block_ty); }
            Instr::If    { block_ty } => { self.write_u8(0x04); self.write_u8(*block_ty); }

            Instr::Br   (l) => { self.write_u8(0x0C); self.write_leb128_u32(*l); }
            Instr::BrIf (l) => { self.write_u8(0x0D); self.write_leb128_u32(*l); }

            Instr::Call(idx)  => { self.write_u8(0x10); self.write_leb128_u32(*idx); }

            Instr::LocalGet(i) => { self.write_u8(0x20); self.write_leb128_u32(*i); }
            Instr::LocalSet(i) => { self.write_u8(0x21); self.write_leb128_u32(*i); }
            Instr::LocalTee(i) => { self.write_u8(0x22); self.write_leb128_u32(*i); }

            Instr::I32Const(n) => { self.write_u8(0x41); self.write_leb128_i32(*n); }
            Instr::I64Const(n) => { self.write_u8(0x42); self.write_leb128_i32(*n); }
            Instr::F32Const(f) => { self.write_u8(0x43); self.buf.extend_from_slice(&f.to_le_bytes()); }
            Instr::F64Const(f) => { self.write_u8(0x44); let d = *f as f64; self.buf.extend_from_slice(&d.to_le_bytes()); }

            Instr::I32Load   { offset, align } => { self.write_u8(0x28); self.write_leb128_u32(*align); self.write_leb128_u32(*offset); }
            Instr::I32Store  { offset, align } => { self.write_u8(0x36); self.write_leb128_u32(*align); self.write_leb128_u32(*offset); }
            Instr::I32Load8S { offset, align } => { self.write_u8(0x2C); self.write_leb128_u32(*align); self.write_leb128_u32(*offset); }
            Instr::I32Load8U { offset, align } => { self.write_u8(0x2D); self.write_leb128_u32(*align); self.write_leb128_u32(*offset); }
            Instr::I32Store8 { offset, align } => { self.write_u8(0x3A); self.write_leb128_u32(*align); self.write_leb128_u32(*offset); }

            Instr::I32Add  => self.write_u8(0x6A), Instr::I32Sub  => self.write_u8(0x6B),
            Instr::I32Mul  => self.write_u8(0x6C), Instr::I32DivS => self.write_u8(0x6D),
            Instr::I32DivU => self.write_u8(0x6E), Instr::I32RemS => self.write_u8(0x6F),
            Instr::I32RemU => self.write_u8(0x70),
            Instr::I32And  => self.write_u8(0x71), Instr::I32Or   => self.write_u8(0x72),
            Instr::I32Xor  => self.write_u8(0x73),
            Instr::I32Shl  => self.write_u8(0x74), Instr::I32ShrS => self.write_u8(0x75),
            Instr::I32ShrU => self.write_u8(0x76),
            Instr::I32Eqz  => self.write_u8(0x45), Instr::I32Eq   => self.write_u8(0x46),
            Instr::I32Ne   => self.write_u8(0x47),
            Instr::I32LtS  => self.write_u8(0x48), Instr::I32LtU  => self.write_u8(0x49),
            Instr::I32GtS  => self.write_u8(0x4A), Instr::I32GtU  => self.write_u8(0x4B),
            Instr::I32LeS  => self.write_u8(0x4C), Instr::I32LeU  => self.write_u8(0x4D),
            Instr::I32GeS  => self.write_u8(0x4E), Instr::I32GeU  => self.write_u8(0x4F),

            Instr::I64Add  => self.write_u8(0x7C), Instr::I64Sub  => self.write_u8(0x7D),
            Instr::I64Mul  => self.write_u8(0x7E),
            Instr::I64And  => self.write_u8(0x83), Instr::I64Or   => self.write_u8(0x84),
            Instr::I64Xor  => self.write_u8(0x85),
            Instr::I64Eqz  => self.write_u8(0x50), Instr::I64Eq   => self.write_u8(0x51),
            Instr::I64Ne   => self.write_u8(0x52),

            Instr::F32Add  => self.write_u8(0x92), Instr::F32Sub  => self.write_u8(0x93),
            Instr::F32Mul  => self.write_u8(0x94), Instr::F32Div  => self.write_u8(0x95),
            Instr::F64Add  => self.write_u8(0xA0), Instr::F64Sub  => self.write_u8(0xA1),
            Instr::F64Mul  => self.write_u8(0xA2), Instr::F64Div  => self.write_u8(0xA3),
        }
    }

    pub fn finish(self) -> Vec<u8> { self.buf }
}

/// Compress a local list into (count, type) run-length pairs.
fn compress_locals(locals: &[Local]) -> Vec<(u32, ValType)> {
    let mut groups: Vec<(u32, ValType)> = Vec::new();
    for local in locals {
        if let Some(last) = groups.last_mut() {
            if last.1 == local.ty { last.0 += 1; continue; }
        }
        groups.push((1, local.ty));
    }
    groups
}

// ---------------- Run function ----------------
pub fn run(argv: &[&str]) {
    if argv.is_empty() {
        crate::println!("usage: asm <name>.wat");
        return;
    }

    let name = argv[0];

    if !is_wat_file(name) {
        crate::println!("asm: {}: expected .wat file", name);
        return;
    }

    let fat_path  = resolve_fat_path(name);
    let wasm_name = wat_to_wasm_name(name);
    let wasm_path = resolve_fat_path(&wasm_name);

    let data = match crate::fs::fat::fat_read_path(&fat_path) {
        None    => { crate::println!("asm: {}: no such file", name); return; }
        Some(d) => d,
    };

    let wat_source = match core::str::from_utf8(&data) {
        Ok(s)  => s,
        Err(_) => { crate::println!("asm: {}: invalid utf-8", name); return; }
    };

    crate::print!("{}", wat_source);
    if !wat_source.ends_with('\n') { crate::println!(); }

    crate::println!("asm: parsing WAT source");
    let mut parser = Parser::new(wat_source);

    crate::println!("asm: parsing module");
    let module = parser.parse_module();

    crate::println!(
        "asm: {} type(s), {} import(s), {} func(s), {} memory, {} export(s), {} data seg(s)",
        module.types.len(), module.imports.len(), module.funcs.len(),
        module.memories.len(), module.exports.len(), module.data.len()
    );

    crate::println!("asm: emitting WASM binary");
    let mut emitter = WasmEmitter::new();
    emitter.emit_module(&module);
    let wasm_bytes = emitter.finish();
    let len = wasm_bytes.len();

    let fat_ok = crate::fs::fat::fat_write_file(&wasm_path, &wasm_bytes);

    let mem_ok = match crate::fs::alloc_write_buf(&wasm_bytes) {
        Some(static_buf) => {
            crate::fs::register_file(&wasm_name, static_buf);
            true
        }
        None => false,
    };

    if fat_ok || mem_ok {
        crate::println!("saved {} ({} bytes)", wasm_name, len);
    } else {
        crate::println!("asm: save failed");
    }
}

fn resolve_fat_path(name: &str) -> String {
    if name.starts_with('/') {
        String::from(name)
    } else {
        let cwd = crate::shell::get_cwd();
        if cwd == "/" {
            String::from(name)
        } else {
            let dir = cwd.trim_start_matches('/');
            let mut s = String::from(dir);
            s.push('/');
            s.push_str(name);
            s
        }
    }
}

fn is_wat_file(name: &str) -> bool { name.ends_with(".wat") }

fn wat_to_wasm_name(name: &str) -> String {
    if name.ends_with(".wat") {
        let mut out = String::from(&name[..name.len() - 4]);
        out.push_str(".wasm");
        out
    } else {
        String::from(name)
    }
}