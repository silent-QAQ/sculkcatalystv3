fn main() {
    if let Err(error) = sculk_platform_bots::run(sculk_platform_bots::Platform::Douyin) {
        eprintln!("sculk-douyin: {error}");
        std::process::exit(1);
    }
}
