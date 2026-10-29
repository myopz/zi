;; Minimal zi WASM ABI v1 guest.
;; Build: wat2wasm hello.wat -o extension.wasm
;; Or run the crate smoke test (feature wasm-extensions) which assembles this WAT in-process.
(module
  (import "zi_host" "host_api_version" (func $api (result i32)))
  (import "zi_host" "register_command" (func $reg_cmd (param i32 i32)))
  (import "zi_host" "subscribe_hook" (func $sub (param i32 i32)))
  (import "zi_host" "ui_notify" (func $notify (param i32 i32)))
  (import "zi_host" "emit_log" (func $log (param i32 i32)))

  (memory (export "memory") 1)
  (global $heap (mut i32) (i32.const 2048))

  ;; Static payloads (lengths must match call sites below)
  (data (i32.const 0) "{\"name\":\"hello\",\"summary\":\"ABI v1 hello command\"}")
  (data (i32.const 64) "{\"event\":\"agent_end\"}")
  (data (i32.const 128) "{\"level\":\"info\",\"message\":\"hello extension init\"}")
  (data (i32.const 192) "{\"message\":\"hello from wasm agent_end\"}")
  (data (i32.const 320) "hello from /hello")

  (func (export "zi_alloc") (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $heap))
    (global.set $heap (i32.add (global.get $heap) (local.get $n)))
    (local.get $p)
  )

  (func (export "zi_extension_init") (result i32)
    (drop (call $api))
    (call $log (i32.const 128) (i32.const 49))
    (call $reg_cmd (i32.const 0) (i32.const 49))
    (call $sub (i32.const 64) (i32.const 21))
    (i32.const 0)
  )

  (func (export "zi_on_hook") (param $ptr i32) (param $len i32) (result i32)
    (call $notify (i32.const 192) (i32.const 39))
    (i32.const 0)
  )

  (func (export "zi_call_command") (param $ptr i32) (param $len i32) (result i32)
    ;; length-prefixed result at 400
    (i32.store (i32.const 400) (i32.const 17))
    (memory.copy (i32.const 404) (i32.const 320) (i32.const 17))
    (i32.const 400)
  )
)
