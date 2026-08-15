fn main() {
    println!("cargo:rerun-if-changed=../../proto/bean_key.proto");
    prost_build::compile_protos(&["../../proto/bean_key.proto"], &["../../proto"])
        .expect("beanKey protocol generation failed");
}
