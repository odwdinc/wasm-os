/// Tests for every app under userland/.
///
/// Each test loads the pre-compiled .wasm, calls "main", and asserts the
/// captured output (or lack thereof) matches the expected behaviour.
///
/// Run: cd wasm-test && cargo test userland

use wasm_test::run_app;

// ── hello ─────────────────────────────────────────────────────────────────────

#[test]
fn hello_prints_greeting() {
    let wasm = include_bytes!("../../userland/hello/hello.wasm");
    let out  = run_app(wasm, &[]).expect("hello failed");
    assert_eq!(out, "Hello from WASM!\n");
}

// ── greet ─────────────────────────────────────────────────────────────────────

#[test]
fn greet_prints_greeting() {
    let wasm = include_bytes!("../../userland/greet/greet.wasm");
    let out  = run_app(wasm, &[]).expect("greet failed");
    assert_eq!(out, "Greetings from the second module!\n");
}

// ── fib ───────────────────────────────────────────────────────────────────────
//
// fib::main(n) calls print_int(fib(n)), so captured output is "result\n".

fn fib_output(n: i32) -> i64 {
    let wasm = include_bytes!("../../userland/fib/fib.wasm");
    let out  = run_app(wasm, &[n]).expect("fib failed");
    out.trim_end_matches('\n').parse::<i64>().expect("expected integer output")
}

#[test]
fn fib_base_cases() {
    assert_eq!(fib_output(0), 0);
    assert_eq!(fib_output(1), 1);
    assert_eq!(fib_output(2), 1);
}

#[test]
fn fib_small_values() {
    assert_eq!(fib_output(5),  5);
    assert_eq!(fib_output(7),  13);
    assert_eq!(fib_output(10), 55);
}

#[test]
fn fib_larger_values() {
    assert_eq!(fib_output(20), 6765);
    assert_eq!(fib_output(25), 75025);
}

// ── primes ────────────────────────────────────────────────────────────────────
//
// primes::main() runs a sieve of Eratosthenes for 2..=50 and prints each
// prime followed by a space, then a newline.

#[test]
fn primes_header_and_list() {
    let wasm = include_bytes!("../../userland/primes/primes.wasm");
    let out  = run_app(wasm, &[]).expect("primes failed");

    assert!(out.starts_with("Primes up to 50: "),
            "unexpected header: {out:?}");

    let body = out
        .strip_prefix("Primes up to 50: ").unwrap()
        .trim();

    let found: Vec<u32> = body.split_whitespace()
        .map(|s| s.parse().expect("expected number"))
        .collect();

    assert_eq!(
        found,
        [2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47],
        "prime list mismatch",
    );
}

// ── collatz ───────────────────────────────────────────────────────────────────
//
// collatz::main(n) runs the Collatz sequence until it reaches 1.
// Print calls inside the loop are commented out in the WAT, so the only
// thing we can assert is that the function terminates without error.

#[test]
fn collatz_terminates_for_default() {
    // n=0 → the WAT defaults to 27 (111-step sequence).
    let wasm = include_bytes!("../../userland/collatz/collatz.wasm");
    let out  = run_app(wasm, &[0]).expect("collatz(27) should terminate");
    assert!(out.starts_with("Collatz sequence:\n"),
            "unexpected header: {out:?}");
    let body = out
        .strip_prefix("Collatz sequence:\n").unwrap()
        .trim();
    let expected = "27\n82\n41\n124\n62\n31\n94\n47\n142\n71\n214\n107\n322\n161\n484\n242\n121\n364\n182\n91\n274\n137\n412\n206\n103\n310\n155\n466\n233\n700\n350\n175\n526\n263\n790\n395\n1186\n593\n1780\n890\n445\n1336\n668\n334\n167\n502\n251\n754\n377\n1132\n566\n283\n850\n425\n1276\n638\n319\n958\n479\n1438\n719\n2158\n1079\n3238\n1619\n4858\n2429\n7288\n3644\n1822\n911\n2734\n1367\n4102\n2051\n6154\n3077\n9232\n4616\n2308\n1154\n577\n1732\n866\n433\n1300\n650\n325\n976\n488\n244\n122\n61\n184\n92\n46\n23\n70\n35\n106\n53\n160\n80\n40\n20\n10\n5\n16\n8\n4\n2\n1";
    assert_eq!(body, expected);
}

#[test]
fn collatz_terminates_for_power_of_two() {
    // Powers of two halve every step: 16 → 8 → 4 → 2 → 1.
    let wasm = include_bytes!("../../userland/collatz/collatz.wasm");
    let out  = run_app(wasm, &[16]).expect("collatz(16) should terminate");
    assert!(out.starts_with("Collatz sequence:\n"),
            "unexpected header: {out:?}");
    let body = out
        .strip_prefix("Collatz sequence:\n").unwrap()
        .trim();
    let expected =
        "16\n\
         8\n\
         4\n\
         2\n\
         1";
    assert_eq!(body, expected);
}

#[test]
fn collatz_terminates_for_one() {
    // n=1 is already the fixed point — the loop body never executes.
    let wasm = include_bytes!("../../userland/collatz/collatz.wasm");
    let out  = run_app(wasm, &[1]).expect("collatz(1) should terminate immediately");
    assert!(out.starts_with("Collatz sequence:\n"),
            "unexpected header: {out:?}");
    let body = out
        .strip_prefix("Collatz sequence:\n").unwrap()
        .trim();
    let expected = "1";
    assert_eq!(body, expected);
}

// ── counter ───────────────────────────────────────────────────────────────────
//
// counter::main() counts from 1 to 5, printing "counter: N\n" each iteration
// and calling sleep_ms(200) between steps.  run_app resumes through yields
// so the full output is captured.

#[test]
fn counter_prints_five_lines() {
    let wasm = include_bytes!("../../userland/counter/counter.wasm");
    let out  = run_app(wasm, &[]).expect("counter failed");

    let expected =
        "counter: 1\n\
         counter: 2\n\
         counter: 3\n\
         counter: 4\n\
         counter: 5\n";

    assert_eq!(out, expected);
}
