// Utility functions for embedded Claude instances

use std::net::TcpListener as StdTcpListener;

/// Find an available port in the given range
///
/// # Arguments
/// * `start` - Start of port range (inclusive)
/// * `end` - End of port range (inclusive)
///
/// # Returns
/// * `Ok(port)` - First available port in range
/// * `Err(message)` - No available ports found
pub fn find_available_port(start: u16, end: u16) -> Result<u16, String> {
    for port in start..=end {
        if StdTcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }

    Err(format!("No available ports in range {}-{}", start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_available_port() {
        // Should find at least one port in a reasonable range
        let result = find_available_port(50000, 50100);
        assert!(result.is_ok());

        let port = result.unwrap();
        assert!(port >= 50000 && port <= 50100);
    }

    #[test]
    fn test_find_available_port_no_ports() {
        // Bind all ports in a small range
        let _listener1 = StdTcpListener::bind(("127.0.0.1", 60000)).unwrap();
        let _listener2 = StdTcpListener::bind(("127.0.0.1", 60001)).unwrap();

        let result = find_available_port(60000, 60001);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No available ports"));
    }
}
