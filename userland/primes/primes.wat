(module
  ;; Host import: print(ptr: i32, len: i32)
  (import "env" "print" (func $print (param i32 i32)))

  (memory 1)

  ;; Memory layout:
  ;;   0..16  "Primes up to 50: " (17 bytes, ends with space)
  ;;   17     '\n'
  ;;   18     ' '
  ;;   100..110  itoa scratch buffer (11 bytes)
  ;;   200..250  sieve[0..50]
  (data (i32.const 0)  "Primes up to 50: ")
  (data (i32.const 17) "\n ")

  ;; Print a non-negative i32 as decimal (no newline) using itoa.
  ;; Digits are written right-to-left into mem[100..111], then printed.
  (func $print_num (param $n i32)
    (local $pos   i32)
    (local $digit i32)
    (local.set $pos (i32.const 111))
    (block $done
      (loop $digits
        (local.set $digit (i32.rem_u (local.get $n) (i32.const 10)))
        (local.set $n     (i32.div_u (local.get $n) (i32.const 10)))
        (local.set $pos   (i32.sub   (local.get $pos) (i32.const 1)))
        (i32.store8 (local.get $pos)
                    (i32.add (i32.const 48) (local.get $digit)))
        (br_if $done (i32.eqz (local.get $n)))
        (br $digits)
      )
    )
    (call $print (local.get $pos) (i32.sub (i32.const 111) (local.get $pos)))
  )

  (func (export "main")
    (local $i i32)
    (local $j i32)

    ;; ── init sieve[0..50] at mem[200..250] = 1 ─────────────────────────
    (local.set $i (i32.const 0))
    (block $init_done
      (loop $init
        (br_if $init_done (i32.gt_s (local.get $i) (i32.const 50)))
        (i32.store8 (i32.add (i32.const 200) (local.get $i)) (i32.const 1))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $init)
      )
    )
    ;; 0 and 1 are not prime
    (i32.store8 (i32.const 200) (i32.const 0))
    (i32.store8 (i32.const 201) (i32.const 0))

    ;; ── sieve: cross out composites for i = 2..7 ───────────────────────
    (local.set $i (i32.const 2))
    (block $sieve_done
      (loop $sieve
        (br_if $sieve_done (i32.gt_s (local.get $i) (i32.const 7)))
        (block $sieve_skip
          ;; skip if sieve[i] == 0 (already composite)
          (br_if $sieve_skip
            (i32.eqz (i32.load8_u (i32.add (i32.const 200) (local.get $i)))))
          ;; mark multiples of i starting at i*i
          (local.set $j (i32.mul (local.get $i) (local.get $i)))
          (block $mark_done
            (loop $mark
              (br_if $mark_done (i32.gt_s (local.get $j) (i32.const 50)))
              (i32.store8 (i32.add (i32.const 200) (local.get $j)) (i32.const 0))
              (local.set $j (i32.add (local.get $j) (local.get $i)))
              (br $mark)
            )
          )
        )
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $sieve)
      )
    )

    ;; ── print "Primes up to 50: " ───────────────────────────────────────
    (call $print (i32.const 0) (i32.const 17))

    ;; ── print each prime followed by a space ────────────────────────────
    (local.set $i (i32.const 2))
    (block $print_done
      (loop $print_loop
        (br_if $print_done (i32.gt_s (local.get $i) (i32.const 50)))
        (block $not_prime
          (br_if $not_prime
            (i32.eqz (i32.load8_u (i32.add (i32.const 200) (local.get $i)))))
          (call $print_num (local.get $i))
          (call $print (i32.const 18) (i32.const 1))  ;; ' '
        )
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $print_loop)
      )
    )

    ;; ── print newline ────────────────────────────────────────────────────
    (call $print (i32.const 17) (i32.const 1))
  )
)
