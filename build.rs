fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/fruit-basket.ico");
    basket_build::windows_icon("assets/fruit-basket.ico", "Fruit Basket", "Fruit Basket, the launcher for the Fruit Basket emulators");
}
