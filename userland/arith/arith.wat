(module
  (import "env" "print_int" (func $print_int (param i32)))

  (memory 1)

  ;; add(a, b) = a + b  — straight-line, no control flow
  (func $add (param $a i32) (param $b i32) (result i32)
    (i32.add (local.get $a) (local.get $b))
  )

  ;; mul_add(a, b, c) = a * b + c
  (func $mul_add (param $a i32) (param $b i32) (param $c i32) (result i32)
    (i32.add (i32.mul (local.get $a) (local.get $b)) (local.get $c))
  )

  ;; bitops(x) = ((x & 0xFF) | 0x100) ^ 0x55
  (func $bitops (param $x i32) (result i32)
    (local $t i32)
    (local.set $t (i32.and (local.get $x) (i32.const 255)))
    (local.set $t (i32.or  (local.get $t) (i32.const 256)))
    (i32.xor (local.get $t) (i32.const 85))
  )

  ;; shift_ops(x) = (x << 3) + (x >> 1)
  (func $shift_ops (param $x i32) (result i32)
    (i32.add
      (i32.shl  (local.get $x) (i32.const 3))
      (i32.shr_u (local.get $x) (i32.const 1))
    )
  )

  ;; Entry point: exercise all four functions and print each result.
  (func (export "main") (param $n i32)
    (call $print_int (call $add      (local.get $n) (i32.const 10)))
    (call $print_int (call $mul_add  (local.get $n) (i32.const 3) (i32.const 7)))
    (call $print_int (call $bitops   (local.get $n)))
    (call $print_int (call $shift_ops (local.get $n)))
  )
)
