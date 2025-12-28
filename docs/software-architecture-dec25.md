# Homegate Software Architecture

## Architecture Pattern: Modular Monolith with Hexagonal Architecture

This codebase implements a **Modular Monolith** following **Hexagonal Architecture** (also known as **Ports & Adapters**) principles, with influences from **Clean Architecture**.

### Pattern Names

| Pattern | Description |
|---------|-------------|
| **Modular Monolith** | Independent, self-contained modules (sms_verification, ln_verification) within a single deployable unit |
| **Hexagonal Architecture** | Clear separation between business logic and external systems via ports (interfaces) and adapters (implementations) |
| **Clean Architecture** | Layered structure with dependency inversion (dependencies point inward toward domain) |

## High-Level Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                  INBOUND ADAPTERS (HTTP)                    │
│      sms_verification/http.rs, ln_verification/http.rs      │
└────────────────────────────┬────────────────────────────────┘
                             │
┌────────────────────────────▼────────────────────────────────┐
│                  APPLICATION LAYER                          │
│   SmsVerificationService, LnVerificationService             │
│   (Business logic, orchestration, use cases)                │
└────────────────────────────┬────────────────────────────────┘
                             │
┌────────────────────────────▼────────────────────────────────┐
│                    DOMAIN LAYER                             │
│   Value Objects: PhoneNumber, Code, PaymentHash             │
│   (Core types with validation, no dependencies)             │
└────────────────────────────┬────────────────────────────────┘
                             │
┌────────────────────────────▼────────────────────────────────┐
│                 OUTBOUND ADAPTERS                           │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────┐  │
│  │  Repositories   │  │  External APIs  │  │  SQL Layer  │  │
│  │  (Data Access)  │  │  (PreludeAPI,   │  │  (SqlDb,    │  │
│  │                 │  │  PhoenixdAPI,   │  │  sea-query) │  │
│  │                 │  │  HomeserverAPI) │  │             │  │
│  └─────────────────┘  └─────────────────┘  └─────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

## Directory Structure

```
src/
├── main.rs                    # Entry point, bootstrap
├── infrastructure/            # Cross-cutting infrastructure
│   ├── config.rs              # Environment configuration
│   ├── http/                  # HTTP server, extractors, errors
│   └── sql/                   # Database abstraction, migrations
├── shared/                    # Shared services across modules
│   ├── homeserver_admin_api.rs
│   └── homeserver_observer.rs
├── sms_verification/          # SMS verification module
│   ├── mod.rs
│   ├── http.rs                # Inbound adapter (HTTP handlers)
│   ├── service.rs             # Application layer (business logic)
│   ├── repository.rs          # Outbound adapter (data access)
│   ├── types.rs               # Domain layer (value objects)
│   ├── prelude_api.rs         # Outbound adapter (SMS provider)
│   └── error.rs               # Module-specific errors
└── ln_verification/           # Lightning verification module
    ├── mod.rs
    ├── http.rs                # Inbound adapter
    ├── service.rs             # Application layer
    ├── repository.rs          # Outbound adapter
    ├── phoenixd_api/          # Outbound adapter (Lightning provider)
    └── error.rs               # Module-specific errors
```

## Key Design Patterns

### 1. Ports & Adapters (Hexagonal)

**Inbound Ports** (driving the application):
- HTTP handlers in `http.rs` receive requests and delegate to services

**Outbound Ports** (driven by the application):
- `PreludeAPI` - SMS provider integration
- `PhoenixdAPI` - Lightning Network provider
- `HomeserverAdminAPI` - Homeserver integration
- `Repository` - Database abstraction

### 2. Repository Pattern

```rust
// Repositories abstract data access with static methods
SmsVerificationRepository::create_verification(executor, entity)
SmsVerificationRepository::mark_verified(executor, id)
LnVerificationRepository::get_verification_by_payment_hash(executor, hash)
```

### 3. Service Layer Pattern

```rust
// Services encapsulate business logic and orchestrate dependencies
impl SmsVerificationService {
    pub async fn create_verification(&self, phone: PhoneNumber) -> Result<...>
    pub async fn validate_code(&self, request: ValidateCodeRequest) -> Result<...>
}
```

### 4. Value Object Pattern

```rust
// Domain types with built-in validation
pub struct PhoneNumber(String);  // E.164 format validation
pub struct Code(String);         // 6-digit validation
pub struct PaymentHash(String);  // Lightning payment hash
```

### 5. App State Pattern (Dependency Injection)

```rust
// Axum state for dependency injection
#[derive(Clone)]
pub struct AppState {
    pub db: SqlDb,
    pub sms_verification: SmsVerificationService,
}

// Used in handlers
async fn handler(State(state): State<AppState>) -> Result<...>
```

## Module Independence

Each verification module is self-contained with its own:
- HTTP handlers (inbound adapter)
- Service (business logic)
- Repository (data access)
- External API client (outbound adapter)
- Error types
- Domain types

This makes modules:
- **Independently testable** - Mock external APIs with Wiremock
- **Easily extensible** - Add new modules following the same pattern
- **Potentially extractable** - Could become microservices if needed

## Dependency Flow

```
HTTP Handler
    ↓ depends on
Service (business logic)
    ↓ depends on
Repository + External APIs
    ↓ depends on
Infrastructure (SqlDb, HTTP clients)
```

Dependencies always point **inward** toward the domain, following the Dependency Inversion Principle.

## Error Handling

Layered error types with automatic conversion:

```
PhoenixdError → LnVerificationError → HTTP Response
PreludeError  → SmsVerificationError → HTTP Response
```

Each layer defines its own error type, and `From<T>` implementations enable clean error propagation.

## Why This Architecture?

| Benefit | Description |
|---------|-------------|
| **Testability** | External systems are abstracted behind adapters, easily mockable |
| **Maintainability** | Clear boundaries prevent coupling between modules |
| **Flexibility** | Easy to swap implementations (different SMS provider, different DB) |
| **Scalability** | Modules can be extracted to microservices if needed |
| **Clarity** | New developers can understand module structure quickly |

## References

- [Hexagonal Architecture (Alistair Cockburn)](https://alistair.cockburn.us/hexagonal-architecture/)
- [Clean Architecture (Robert C. Martin)](https://blog.cleancoder.com/uncle-bob/2012/08/13/the-clean-architecture.html)
- [Modular Monolith (Kamil Grzybek)](https://www.kamilgrzybek.com/blog/posts/modular-monolith-primer)
