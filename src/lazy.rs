use std::sync::OnceLock;

struct Lazy<T>(OnceLock<T>);

impl<T> Lazy<T>{
    pub fn new(init: FnOnce() -> T) -> Self<T>{
        Self{
            
        }
    }
}

// static ROOT_DIR: Lazy<PathBuf> = Lazy::new(||std::env::current_dir().unwrap());