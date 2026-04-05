/// Example: i32 arithmetic operations.
///
/// Test plan → Rust mapping:
///   Given a WAT source file (or inline string), compile it once, then assert
///   the return value of each named export for every (args → expected) pair.
///
/// This mirrors a WAST `(assert_return (invoke "name" …) …)` block without
/// needing a full WAST parser.

use wasm_test::run_wasm;

// ── Compile WAT, call a function, return the i64 result (panics on error) ──
fn call(wat_src: &str, func: &str, args: &[i32]) -> i64 {
    let wasm = wat::parse_str(wat_src).expect("WAT parse failed");
    run_wasm(&wasm, func, &args)
        .expect("run_wasm failed")
        .expect("function returned no value")
}

// ── WAT module under test ────────────────────────────────────────────────────

const I32_MODULE: &str = r#"
(module
  (func (export "add")   (param i32 i32) (result i32) local.get 0 local.get 1 i32.add)
  (func (export "sub")   (param i32 i32) (result i32) local.get 0 local.get 1 i32.sub)
  (func (export "mul")   (param i32 i32) (result i32) local.get 0 local.get 1 i32.mul)
  (func (export "div_s") (param i32 i32) (result i32) local.get 0 local.get 1 i32.div_s)
  (func (export "div_u") (param i32 i32) (result i32) local.get 0 local.get 1 i32.div_u)
  (func (export "rem_s") (param i32 i32) (result i32) local.get 0 local.get 1 i32.rem_s)
  (func (export "and")   (param i32 i32) (result i32) local.get 0 local.get 1 i32.and)
  (func (export "or")    (param i32 i32) (result i32) local.get 0 local.get 1 i32.or)
  (func (export "xor")   (param i32 i32) (result i32) local.get 0 local.get 1 i32.xor)
  (func (export "shl")   (param i32 i32) (result i32) local.get 0 local.get 1 i32.shl)
  (func (export "shr_s") (param i32 i32) (result i32) local.get 0 local.get 1 i32.shr_s)
  (func (export "shr_u") (param i32 i32) (result i32) local.get 0 local.get 1 i32.shr_u)
  (func (export "clz")   (param i32)     (result i32) local.get 0 i32.clz)
  (func (export "ctz")   (param i32)     (result i32) local.get 0 i32.ctz)
  (func (export "eqz")   (param i32)     (result i32) local.get 0 i32.eqz)
  (func (export "eq")    (param i32 i32) (result i32) local.get 0 local.get 1 i32.eq)
  (func (export "ne")    (param i32 i32) (result i32) local.get 0 local.get 1 i32.ne)
  (func (export "lt_s")  (param i32 i32) (result i32) local.get 0 local.get 1 i32.lt_s)
  (func (export "gt_s")  (param i32 i32) (result i32) local.get 0 local.get 1 i32.gt_s)
)
"#;

// helper: cast i32 bit-pattern to i64 (sign-extends for negative results)
fn i32(n: i32) -> i64 { n as i64 }

// ── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn add() {
    assert_eq!(call(I32_MODULE, "add", &[1, 1]),       i32(2));
    assert_eq!(call(I32_MODULE, "add", &[1, 0]),       i32(1));
    assert_eq!(call(I32_MODULE, "add", &[-1, -1]),     i32(-2));
    assert_eq!(call(I32_MODULE, "add", &[i32::MAX, 1]),i32(i32::MIN)); // wraps
}

#[test]
fn sub() {
    assert_eq!(call(I32_MODULE, "sub", &[1, 1]),   i32(0));
    assert_eq!(call(I32_MODULE, "sub", &[0, 1]),   i32(-1));
    assert_eq!(call(I32_MODULE, "sub", &[i32::MIN, -1]), i32(i32::MIN.wrapping_sub(-1)));
}

#[test]
fn mul() {
    assert_eq!(call(I32_MODULE, "mul", &[3, 4]),    i32(12));
    assert_eq!(call(I32_MODULE, "mul", &[-3, 4]),   i32(-12));
    assert_eq!(call(I32_MODULE, "mul", &[-3, -4]),  i32(12));
    assert_eq!(call(I32_MODULE, "mul", &[0x10000, 0x10000]), i32(0));
}

#[test]
fn div_s() {
    assert_eq!(call(I32_MODULE, "div_s", &[7, 2]),   i32(3));
    assert_eq!(call(I32_MODULE, "div_s", &[-7, 2]),  i32(-3));
    assert_eq!(call(I32_MODULE, "div_s", &[7, -2]),  i32(-3));
    assert_eq!(call(I32_MODULE, "div_s", &[-7, -2]), i32(3));
}

#[test]
fn div_s_by_zero_traps() {
    let wasm = wat::parse_str(I32_MODULE).unwrap();
    assert!(run_wasm(&wasm, "div_s", &[1, 0]).is_err());
}

#[test]
fn bitwise() {
    assert_eq!(call(I32_MODULE, "and", &[0xF0, 0x0F]), i32(0x00));
    assert_eq!(call(I32_MODULE, "or",  &[0xF0, 0x0F]), i32(0xFF));
    assert_eq!(call(I32_MODULE, "xor", &[0xFF, 0x0F]), i32(0xF0));
}

#[test]
fn shifts() {
    assert_eq!(call(I32_MODULE, "shl",   &[1, 4]),  i32(16));
    assert_eq!(call(I32_MODULE, "shr_s", &[-16, 4]), i32(-1));
    assert_eq!(call(I32_MODULE, "shr_u", &[-1, 4]),  i32(0x0FFF_FFFF));
}

#[test]
fn bit_counts() {
    assert_eq!(call(I32_MODULE, "clz", &[0]),          i32(32));
    assert_eq!(call(I32_MODULE, "clz", &[1]),          i32(31));
    assert_eq!(call(I32_MODULE, "clz", &[0x8000_0000u32 as i32]), i32(0));
    assert_eq!(call(I32_MODULE, "ctz", &[0]),          i32(32));
    assert_eq!(call(I32_MODULE, "ctz", &[1]),          i32(0));
    assert_eq!(call(I32_MODULE, "ctz", &[0x0000_0010]), i32(4));
}

#[test]
fn comparisons() {
    assert_eq!(call(I32_MODULE, "eqz",  &[0]),      1);
    assert_eq!(call(I32_MODULE, "eqz",  &[1]),      0);
    assert_eq!(call(I32_MODULE, "eq",   &[1, 1]),   1);
    assert_eq!(call(I32_MODULE, "ne",   &[1, 2]),   1);
    assert_eq!(call(I32_MODULE, "lt_s", &[-1, 0]),  1);
    assert_eq!(call(I32_MODULE, "gt_s", &[1, 0]),   1);
}
