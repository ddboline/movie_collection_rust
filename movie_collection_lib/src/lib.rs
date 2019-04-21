#[macro_use]
extern crate serde_derive;

#[macro_use]
extern crate log;

pub mod common;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
