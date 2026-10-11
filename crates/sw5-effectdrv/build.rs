fn main() {
    // Los exports COM (DllGetClassObject & cía.) generan LNK4104 con el .def de rustc: es esperable.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg-cdylib=/IGNORE:4104");
    }
}
