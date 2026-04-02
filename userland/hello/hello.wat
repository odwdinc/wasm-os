(module
  ;; Import the host print function: print(ptr: i32, len: i32)
  (import "os" "print" (func $print (param i32 i32)))

  (memory 1)

  ;; String data at offset 0
  (data (i32.const 0) "Hello from WASM!\n")

  (func (export "main")
    i32.const 0   ;; ptr to string
    i32.const 17  ;; length of "Hello from WASM!\n"
    call $print
  )
)
