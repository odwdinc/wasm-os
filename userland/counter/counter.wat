(module
  (import "env" "print"     (func $print     (param i32 i32)))
  (import "env" "print_int" (func $print_int (param i32)))
  (import "env" "sleep_ms"  (func $sleep_ms  (param i32)))

  (memory 1)
  (data (i32.const 0) "counter: ")  ;; 9 bytes at offset 0

  ;; Counts from 1 to 5, printing each value and sleeping 200 ms between
  ;; iterations so the scheduler can interleave other tasks.
  (func (export "main")
    (local $i i32)
    i32.const 1
    local.set $i

    block $break
      loop $loop
        ;; exit once i > 5
        local.get $i
        i32.const 5
        i32.gt_s
        br_if $break

        ;; print "counter: " (no newline)
        i32.const 0
        i32.const 9
        call $print

        ;; print_int(i) — appends the number + newline
        local.get $i
        call $print_int

        ;; i++
        local.get $i
        i32.const 1
        i32.add
        local.set $i

        ;; yield to the scheduler for ~200 ms
        i32.const 200
        call $sleep_ms

        br $loop
      end
    end
  )
)
