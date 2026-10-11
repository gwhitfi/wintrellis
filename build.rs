fn main() {
    println!("cargo:rerun-if-changed=wintrellis.rc");
    println!("cargo:rerun-if-changed=wintrellis.manifest");
    println!("cargo:rerun-if-changed=wintrellis.ico");
    embed_resource::compile("wintrellis.rc", embed_resource::NONE)
        .manifest_required()
        .unwrap();
}
