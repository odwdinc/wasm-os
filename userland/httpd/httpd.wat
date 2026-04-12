(module
  ;; ── Host imports ──────────────────────────────────────────────────────────
  (import "env" "print"      (func $print      (param i32 i32)))
  (import "env" "print_int"  (func $print_int  (param i32)))
  (import "env" "yield"      (func $yield))
  (import "net" "listen"     (func $net_listen  (param i32)         (result i32)))
  (import "net" "accept"     (func $net_accept  (param i32)         (result i32)))
  (import "net" "recv"       (func $net_recv    (param i32 i32 i32) (result i32)))
  (import "net" "send"       (func $net_send    (param i32 i32 i32) (result i32)))
  (import "net" "close"      (func $net_close   (param i32)         (result i32)))

  (memory 1)

  ;; offset 0..1023   — request receive buffer (uninitialised; overwritten on recv)

  ;; offset 1024      — HTTP/1.0 response (91 bytes)
  ;; "HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 26\r\n\r\nHello from WASM-First OS!\n"
  (data (i32.const 1024)
    "HTTP/1.0 200 OK\0d\0a"
    "Content-Type: text/plain\0d\0a"
    "Content-Length: 26\0d\0a"
    "\0d\0a"
    "Hello from WASM-First OS!\0a"
  )
  ;; response length = 17 + 26 + 20 + 2 + 26 = 91

  ;; offset 1200 — "[httpd] listening on :8080\n"  (27 bytes)
  (data (i32.const 1200) "[httpd] listening on :8080\0a")
  ;; offset 1232 — "[httpd] connection\n"         (19 bytes)
  (data (i32.const 1232) "[httpd] connection\0a")
  ;; offset 1264 — "[httpd] response sent\n"      (22 bytes)
  (data (i32.const 1264) "[httpd] response sent\0a")
  ;; offset 1300 — "[httpd] accept fd: "          (19 bytes)
  (data (i32.const 1300) "[httpd] accept fd: ")
  ;; offset 1320 — "[httpd] recv n: "             (16 bytes)
  (data (i32.const 1320) "[httpd] recv n: ")

  ;; ── main ──────────────────────────────────────────────────────────────────
  (func (export "main")
    (local $listen_fd i32)
    (local $conn_fd   i32)
    (local $n         i32)

    ;; net_listen(8080) → listen_fd
    i32.const 8080
    call $net_listen
    local.tee $listen_fd
    i32.const -1
    i32.eq
    if
      return  ;; no network — exit silently
    end

    ;; "[httpd] listening on :8080\n"
    i32.const 1200
    i32.const 27
    call $print

    ;; ── accept loop — serve one connection at a time ───────────────────────
    loop $server
      ;; spin until net_accept returns a valid handle
      block $got_conn
        loop $wait_conn
          local.get $listen_fd
          call $net_accept
          local.tee $conn_fd
          i32.const -1
          i32.ne
          br_if $got_conn   ;; accepted — exit inner loop
          call $yield
          br $wait_conn
        end
      end

      ;; "[httpd] accept fd: " + conn_fd
      i32.const 1300
      i32.const 19
      call $print
      local.get $conn_fd
      call $print_int

      ;; spin until we receive request bytes
      block $got_data
        loop $wait_data
          local.get $conn_fd
          i32.const 0       ;; recv into offset 0
          i32.const 1024    ;; capacity
          call $net_recv
          local.tee $n
          i32.const 0
          i32.gt_s
          br_if $got_data   ;; n > 0 — we have data
          call $yield
          br $wait_data
        end
      end

      ;; "[httpd] recv n: " + n
      i32.const 1320
      i32.const 16
      call $print
      local.get $n
      call $print_int

      ;; send the HTTP response (offset 1024, 91 bytes)
      local.get $conn_fd
      i32.const 1024
      i32.const 91
      call $net_send
      drop

      ;; "[httpd] response sent\n"
      i32.const 1264
      i32.const 22
      call $print

      ;; close the connection
      local.get $conn_fd
      call $net_close
      drop

      ;; yield once before accepting the next connection
      call $yield

      br $server
    end
  )
)
