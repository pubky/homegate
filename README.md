# Homegate

A backend service to gatekeep [Pubky Homeserver](https://github.com/pubky/pubky-core/) signups.

# Run

See `.env.example` for a description of which values you need to put in a `.env` file.

Then: 
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

## Running Tests

Tests require a PostgreSQL connection string for database integration tests:

```bash
# Run all tests
DATABASE_URL=postgres://postgres:postgres@localhost:5432/pubky_homegate?pubky-test=true cargo test

# Run only E2E HTTP tests
DATABASE_URL=postgres://postgres:postgres@localhost:5432/pubky_homegate?pubky-test=true cargo test --lib e2e::
```

### Test Structure

- **Unit Tests**: IP extraction logic (`src/http_server/routes/sms_verification.rs`)
- **Service Tests**: Business logic and database operations (`src/sms_verification/tests.rs`)
- **E2E Tests**: Full HTTP integration tests (`src/e2e/`)
