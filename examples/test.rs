use std::collections::HashMap;
use uuid::Uuid;

trait Protocol {
    fn execute(&self);
}

struct Foo;

impl Foo {
    fn new() -> Self {
        Foo
    }
    
    fn print(&self) {
        println!("Foo");
    }
}

impl Protocol for Foo {
    fn execute(&self) {
        println!("Executing Foo protocol");
    }
}

struct Manager {
    worker: HashMap<Uuid, Box<dyn Protocol>>,
}

fn main() {
    let mut map = HashMap::new();
    map.insert(Uuid::new_v4(), Box::new(Foo::new()) as Box<dyn Protocol>);
    let manager = Manager { worker: map };

    // 示例：遍历并调用协议方法
    for (_, protocol_box) in &manager.worker {
        protocol_box.execute();
    }
}
