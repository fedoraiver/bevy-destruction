fn main() {
    println!("cargo::rustc-check-cfg=cfg(ecs_architecture)");
    println!("cargo::rustc-check-cfg=cfg(feature, values(\"acceptance-test\"))");
    println!("cargo::rustc-cfg=ecs_architecture");
}
