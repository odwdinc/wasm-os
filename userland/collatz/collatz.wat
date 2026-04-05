(module
  (import "env" "print" (func $print (param i32 i32)))
  (import "env" "print_int" (func $print_int (param i32)))
  (memory 1)
  (data (i32.const 0) "Collatz sequence:\n")  ;; 18 bytes

  (func (export "main") (param $n i32)
    ;; if no arg passed, default to 27
    local.get $n
    i32.eqz
    if
      i32.const 27
      local.set $n
    end

    ;; print header
    i32.const 0
    i32.const 18
    call $print

    ;; loop until n == 1
    block $break
      loop $loop
        ;; print current n
        local.get $n
        call $print_int

        ;; if n == 1, break
        local.get $n
        i32.const 1
        i32.eq
        br_if $break

        ;; if n is odd: n = 3n + 1
        ;; if n is even: n = n / 2
        local.get $n
        i32.const 1
        i32.and
        if
          ;; odd
          local.get $n
          i32.const 3
          i32.mul
          i32.const 1
          i32.add
          local.set $n
        else
          ;; even
          local.get $n
          i32.const 1
          i32.shr_s
          local.set $n
        end

        br $loop
      end
    end
  )
)