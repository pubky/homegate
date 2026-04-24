# Homegate

A backend service to gatekeep [Pubky Homeserver](https://github.com/pubky/pubky-core/) signups.

The Pubky social media app uses Homegate to gatekeep signups. Check out https://pubky.app/onboarding/human (use a private browser window if you have an active session).

This service depends on

- [Prelude](https://docs.prelude.so/) as a SMS service provider.
- [PhoenixD](https://github.com/ACINQ/phoenixd) As a Lightning Payment provider.

# Configuration

Homegate is configured via a TOML file. Copy `config.toml.example` to `config.toml` and fill in the required values. Set the `HG_CONFIG_PATH` environment variable to use a different path (defaults to `config.toml` in the working directory).

The `database_url` field must point to an existing PostgreSQL database, e.g.:

```toml
database_url = "postgres://postgres:postgres@localhost:5432/pubky_homegate"
```

Verification routes are **optional** — include an `[sms_verification]`, `[ln_verification]`, or `[ip_verification]` section to enable each one. Omitting a section disables that route entirely.

See `config.toml.example` for the full list of options and defaults.

# Usage

```
cargo run
```

### **Warning**
This code generates a secret which is written to local disk at `/.homegate/pepper.txt`.

If this value is lost then you lose the ability to match phone numbers which have been already verified to phone numbers of new verification requests - this is turn means that the verification limits will not be enforced.

## SMS Verification

We use [Prelude](https://docs.prelude.so/) for SMS verification. Keep in mind that each phone number:
- Has a maximum of 10 verifications.
- Has a single pending verification at a time. Multiple `send_code` calls reuse the existing session.

## Lightning Payment Verification

We use [phoenixd](https://github.com/ACINQ/phoenixd) for Lightning Payment verifications.

## IP Verification

A low-friction alternative to SMS/LN verification. A client POSTs to `/ip_verification` and receives a signup code if their IP has not exceeded the configured weekly/annual limits.

IP-based rate limiting is inherently easy to circumvent (rotating IPs, VPNs). See `src/ip_verification/mod.rs` for detailed security considerations.

Enabled by adding an `[ip_verification]` section to `config.toml`:

```toml
[ip_verification]
max_verifications_per_week = 2   # default
max_verifications_per_year = 4   # default
```

## Running Tests

Tests require a PostgreSQL connection string for database integration tests:

```bash
# Run all tests
DATABASE_URL=postgres://postgres:postgres@localhost:5432/pubky_homegate?pubky-test=true cargo test

# Run only E2E HTTP tests
DATABASE_URL=postgres://postgres:postgres@localhost:5432/pubky_homegate?pubky-test=true cargo test --lib e2e::
```

### Test Structure

- **Unit Tests**: IP extraction logic (`src/infrastructure/http/extractors/request_origin.rs`)
- **Service Tests**: Business logic and database operations (`src/sms_verification/tests.rs`, `src/ip_verification/tests.rs`)
- **E2E Tests**: Full HTTP integration tests (`src/e2e/`)
