# Homegate

A backend service to gatekeep [Pubky Homeserver](https://github.com/pubky/pubky-homeserver/) signups.

The Pubky social media app uses Homegate to gatekeep signups. Check out https://pubky.app/onboarding/human (use a private browser window if you have an active session).

This service depends on

- [Prelude](https://docs.prelude.so/) as a SMS service provider.
- [PhoenixD](https://github.com/ACINQ/phoenixd) As a Lightning Payment provider.
- Google JWKS for Google ID token verification.

# Configuration

Homegate is configured via a TOML file. Copy `config.toml.example` to `~/.homegate/config.toml` and fill in the required values. Use `--data-dir` to specify a different data directory (defaults to `~/.homegate/`).

The `database_url` field must point to an existing PostgreSQL database, e.g.:

```toml
database_url = "postgres://postgres:postgres@localhost:5432/pubky_homegate"
```

Verification routes are **optional** — include an `[sms_verification]`, `[ln_verification]`, `[ip_verification]`, or `[google_verification]` section to enable each one. Omitting a section disables that route entirely.

See `config.toml.example` for the full list of options and defaults.

# Usage

```
cargo run
# Or with a custom data directory:
cargo run -- --data-dir /path/to/data
```

### **Warning**
This code generates a secret which is written to local disk at `~/.homegate/pepper.txt`.

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

Enabled by adding an `[ip_verification]` section to `config.toml`.

## Google Verification

Server-side Google ID token verification for issuing homeserver signup codes. A client POSTs to `/google_verification` with a `googleIdToken` and receives a signup code if the token is valid and the Google identity has not exceeded the configured weekly/annual limits.

Homegate verifies the token signature against Google's JWKS and validates the expected audience, issuer, expiry, and subject. Rate limiting uses a secret-peppered hash of the verified issuer and subject claims; raw Google IDs and emails are not stored.

Enabled by adding a `[google_verification]` section with `google_client_id` to `config.toml`. The JWKS endpoint defaults to Google's well-known URL; the `HOMEGATE_GOOGLE_JWKS_URL` environment variable overrides it if ever needed (e.g. manual testing against a mock).

## Adding a New Verification Provider

Each verification method is a self-contained module (`src/<provider>_verification/`) with its own HTTP router, config section, error enum, and database table. Provider routes are registered conditionally in `src/infrastructure/http/server.rs` based on config presence.

The final step of every low-friction provider — atomically rate-limiting a verified identity and issuing a homeserver signup code — is shared. Do not reimplement it: derive a peppered identity hash with `HasherArgon2id` and delegate to `RateLimitedSignupIssuer` (`src/shared/rate_limited_signup_issuer.rs`). Its module documentation contains the step-by-step recipe; `src/google_verification/` is the reference implementation.

## Running Tests

Tests use the `DATABASE_URL` env var (a `sqlx::test` convention) to provision test database pools. This is separate from the `database_url` field in `config.toml` which is only used at runtime, tests never load `config.toml`.

```bash
# Run all tests
DATABASE_URL=postgres://postgres:postgres@localhost:5432/pubky_homegate?pubky-test=true cargo test

# Run only E2E HTTP tests
DATABASE_URL=postgres://postgres:postgres@localhost:5432/pubky_homegate?pubky-test=true cargo test --lib e2e::
```

### Test Structure

- **Unit Tests**: IP extraction logic (`src/infrastructure/http/extractors/request_origin.rs`)
- **Service Tests**: Business logic and database operations (`src/sms_verification/tests.rs`, `src/ip_verification/tests.rs`, `src/google_verification/tests.rs`)
- **E2E Tests**: Full HTTP integration tests (`src/e2e/`)
