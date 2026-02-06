fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: writer <path> <content>");
        std::process::exit(1);
    }
    let path = &args[1];
    let content = &args[2];

    println!("Process ID: {}", std::process::id());
    println!("Attempting to write to: {}", path);

    match std::fs::write(path, content) {
        Ok(_) => println!("Successfully wrote to {}", path),
        Err(e) => {
            eprintln!("Error writing to {}: {}", path, e);
            std::process::exit(1);
        }
    }
}
