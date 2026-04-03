(module
  (import "env" "print" (func $print (param i32 i32)))

  (memory 1)
  (data (i32.const 0) "Greetings from the second module!\n")

  (func (export "main")
    i32.const 0
    i32.const 34   ;; len("Greetings from the second module!\n")
    call $print
  )
)
