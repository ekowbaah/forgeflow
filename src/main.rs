use forgeflow::Engine;
fn main() {
    let mut e = Engine::default();
    e.submit("demo", "{\"task\":\"hello\"}", 3);
    println!("claimed {:?}", e.claim());
}
