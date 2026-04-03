(module
  (import "env" "print"     (func $print     (param i32 i32)))
  (import "env" "print_int" (func $print_int (param i32)))

  (memory 1)

  ;; Recursive fibonacci.
  (func $fib (param $n i32) (result i32)
    (local $a i32)
    (local $b i32)
    (if (i32.lt_s (local.get $n) (i32.const 2))
      (then (return (local.get $n)))
    )
    (local.set $a (call $fib (i32.sub (local.get $n) (i32.const 1))))
    (local.set $b (call $fib (i32.sub (local.get $n) (i32.const 2))))
    (i32.add (local.get $a) (local.get $b))
  )

  ;; Entry point: compute fib(n) and print the result.
  (func (export "main") (param $n i32)
    (call $print_int (call $fib (local.get $n)))
  )
)
