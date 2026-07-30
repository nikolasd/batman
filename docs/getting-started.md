# BATMAN Getting Started Guide

This guide covers everything you need to get started with BATMAN, from installation to troubleshooting.

## Prerequisites

Before you begin, ensure you have the following installed:

- **Rust** (version 1.70.0 or later)
  - Install via [rustup](https://rustup.rs/): `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
- **Bun** (version 1.0.0 or later)
  - Install via: `curl -fsSL https://bun.sh/install | bash`
- **Git** (version 2.0 or later)
  - Install via your package manager or [git-scm.com](https://git-scm.com/)

## Installation

### Clone the Repository

```bash
git clone https://github.com/your-org/batman.git
cd batman
```

### Install Dependencies

```bash
# Install Rust dependencies
cargo install --path .

# Install Node.js dependencies (if using npm/yarn)
npm install
# or
yarn install
```

## Configuration

### Environment Variables

BATMAN uses environment variables for configuration. The most important ones are:

- `BATMAN_CONFIG_PATH`: Path to your configuration file (default: `~/.batman/config.toml`)
- `BATMAN_LOG_LEVEL`: Logging level (`debug`, `info`, `warn`, `error`)
- `BATMAN_PORT`: Port for the BATMAN server (default: `8080`)

### Configuration File

Create a configuration file at `~/.batman/config.toml` (or the path specified by `BATMAN_CONFIG_PATH`):

```toml
[server]
port = 8080
host = "127.0.0.1"

[database]
url = "sqlite://batman.db"

[logging]
level = "info"
```

## Usage

### Start the Server

```bash
batman serve
```

This starts the BATMAN server with default configuration. To use a custom configuration file:

```bash
batman serve --config /path/to/config.toml
```

### Run Migrations

```bash
batman migrate
```

### Run Tests

```bash
# Run all tests
cargo test

# Run specific test suite
cargo test --package batman-core

# Run with specific features
cargo test --features "feature1,feature2"
```

### CLI Commands

BATMAN provides several CLI commands:

- `batman serve`: Start the server
- `batman migrate`: Run database migrations
- `batman init`: Initialize a new BATMAN project
- `batman version`: Print the BATMAN version

## API Reference

### REST API

BATMAN exposes a REST API on the configured port (default: `8080`).

#### Health Check

```bash
curl http://localhost:8080/health
```

Response:
```json
{
  "status": "ok",
  "version": "1.0.0"
}
```

#### Create a Resource

```bash
curl -X POST http://localhost:8080/resources \
  -H "Content-Type: application/json" \
  -d '{"name": "example", "value": "test"}'
```

Response:
```json
{
  "id": "123e4567-e89b-12d3-a456-426614174000",
  "name": "example",
  "value": "test",
  "created_at": "2023-10-01T12:00:00Z"
}
```

### WebSocket API

BATMAN also supports WebSocket connections for real-time updates.

```javascript
const ws = new WebSocket('ws://localhost:8080/ws');

ws.onmessage = (event) => {
  console.log('Received:', JSON.parse(event.data));
};

ws.send(JSON.stringify({
  type: 'subscribe',
  channel: 'resources'
}));
```

## Testing

### Unit Tests

Unit tests are located in the `tests/` directory and can be run with:

```bash
cargo test
```

### Integration Tests

Integration tests are located in `tests/integration/` and test the full system:

```bash
cargo test --test integration
```

### Benchmark Tests

Performance benchmarks are located in `benches/`:

```bash
cargo bench
```

## Troubleshooting

### Common Issues

#### Port Already in Use

If you see an error like `Address already in use`, another process is using the configured port.

**Solution**:
1. Check what's using the port: `lsof -i :8080` (macOS/Linux) or `netstat -ano | findstr :8080` (Windows)
2. Kill the process or use a different port: `batman serve --port 8081`

#### Database Connection Errors

If you see database-related errors, ensure the database URL in your configuration is correct and the database file is accessible.

**Solution**:
1. Check the `database.url` in your config file
2. Ensure the directory exists and is writable
3. Run migrations: `batman migrate`

#### Permission Errors

If you see permission errors, ensure BATMAN has the necessary permissions to access the configured paths.

**Solution**:
1. Check file permissions: `ls -la ~/.batman/`
2. Adjust permissions if necessary: `chmod 755 ~/.batman/`

### Logging

Enable debug logging to get more detailed error messages:

```bash
batman serve --log-level debug
```

Logs are written to `~/.batman/batman.log` by default.

## Contributing

We welcome contributions! Please see the [CONTRIBUTING.md](CONTRIBUTING.md) file for guidelines.

### Development Setup

1. Clone the repository
2. Install dependencies: `cargo install --path .`
3. Run tests: `cargo test`
4. Make your changes
5. Submit a pull request

### Code Style

- Follow [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- Use `cargo fmt` to format code: `cargo fmt --all`
- Use `cargo clippy` to check for common issues: `cargo clippy --all-targets --all-features -- -D warnings`

## Getting Help

- **Documentation**: [docs.batman.dev](https://docs.batman.dev)
- **Discord**: [discord.gg/batman](https://discord.gg/batman)
- **GitHub Issues**: [github.com/your-org/batman/issues](https://github.com/your-org/batman/issues)
- **Email**: support@batman.dev

## License

BATMAN is released under the [MIT License](LICENSE).
