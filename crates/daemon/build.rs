fn main() {
    println!("cargo:rerun-if-changed=../../proto/beankey.proto");
    prost_build::compile_protos(&["../../proto/beankey.proto"], &["../../proto"])
        .expect("beankey protocol generation failed");
}
