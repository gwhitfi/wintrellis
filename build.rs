fn main() {
    println!("cargo:rerun-if-changed=win-tile.rc");
    println!("cargo:rerun-if-changed=win-tile.manifest");
    embed_resource::compile("win-tile.rc", embed_resource::NONE)
        .manifest_required()
        .unwrap();
}
