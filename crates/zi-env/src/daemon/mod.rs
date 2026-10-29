// Minimal zi-envd server (pi-env daemon subset: hello, exec, read, write, listFiles).

mod serve;

pub use serve::serve;
