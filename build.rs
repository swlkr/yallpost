use cfg_aliases::cfg_aliases;

fn main() {
    cfg_aliases! {
        backend: { all(feature = "ssr") },
        frontend: { all(feature = "csr") }
    }
}
