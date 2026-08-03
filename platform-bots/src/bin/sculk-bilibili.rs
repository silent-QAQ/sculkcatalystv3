fn main() {
    if let Err(error) = sculk_platform_bots::run(sculk_platform_bots::Platform::Bilibili) {
        eprintln!("sculk-bilibili: {error}");
        std::process::exit(1);
    }
}
