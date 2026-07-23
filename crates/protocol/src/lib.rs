#[cfg(test)]
mod tests {
    #[test]
    fn crate_is_wired() {
        assert_eq!(env!("CARGO_PKG_NAME"), "batman-protocol");
    }
}
