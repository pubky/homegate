# Homegate Architecture

A **layered architecture** with **feature modules** sitting atop **shared horizontal infrastructure**. Each module owns its business logic and external API adapters while sharing common resources like database, configuration, and the Homeserver API client.

## Architecture Overview

```
┌────────────────────────────────────────────────────────────────┐
│                        HTTP SERVER                             │
│                       (Axum Router)                            │
└───────────────────────────┬────────────────────────────────────┘
                            │
            ┌───────────────┴───────────────┐
            │                               │
┌───────────▼───────────┐     ┌─────────────▼─────────────┐
│                       │     │                           │
│   SMS Verification    │     │  Lightning Verification   │
│   Module              │     │  Module                   │
│                       │     │                           │
│  ┌─────────────────┐  │     │  ┌─────────────────────┐  │
│  │ HTTP Handlers   │  │     │  │ HTTP Handlers       │  │
│  └────────┬────────┘  │     │  └──────────┬──────────┘  │
│           ▼           │     │             ▼             │
│  ┌─────────────────┐  │     │  ┌─────────────────────┐  │
│  │ Service         │  │     │  │ Service             │  │
│  └────────┬────────┘  │     │  └──────────┬──────────┘  │
│           ▼           │     │             ▼             │
│  ┌─────────────────┐  │     │  ┌─────────────────────┐  │
│  │ Repository      │  │     │  │ Repository          │  │
│  └─────────────────┘  │     │  └─────────────────────┘  │
│           │           │     │             │             │
│  ┌─────────────────┐  │     │  ┌─────────────────────┐  │
│  │ PreludeAPI      │  │     │  │ PhoenixdAPI         │  │
│  │ (SMS Provider)  │  │     │  │ (Lightning Node)    │  │
│  └─────────────────┘  │     │  └─────────────────────┘  │
│                       │     │             │             │
└───────────┬───────────┘     │  ┌─────────────────────┐  │
            │                 │  │ BackgroundSyncer    │  │
            │                 │  │ (Invoice Polling)   │  │
            │                 │  └─────────────────────┘  │
            │                 │                           │
            │                 └─────────────┬─────────────┘
            │                               │
            └───────────────┬───────────────┘
                            │
┌───────────────────────────▼────────────────────────────────────┐
│                    HOMESERVER ADMIN API                        │
│              (Shared client, separate instances)               │
└───────────────────────────┬────────────────────────────────────┘
                            │
┌───────────────────────────▼────────────────────────────────────┐
│                         DATABASE                               │
│                   (Shared SqlDb / PgPool)                      │
└───────────────────────────┬────────────────────────────────────┘
                            │
┌───────────────────────────▼────────────────────────────────────┘
│                          CONFIG                                │
│                (Single EnvConfig instance)                     │
└────────────────────────────────────────────────────────────────┘
```

## Key Characteristics

| Aspect | Description |
|--------|-------------|
| **Layered** | Horizontal layers (Config → Database → Homeserver API → Modules → HTTP) |
| **Shared Infrastructure** | Database pool and config are shared across all modules |
| **Module Ownership** | Each module owns its service logic and external API adapters |
| **Dependency Direction** | Modules depend on shared infrastructure, not on each other |

## Startup Flow

```
main.rs
  │
  ├─► EnvConfig::load()
  │     └─► Loads all configuration from environment variables
  │
  └─► HttpServer::start(config)
        │
        ├─► HomeserverObserver::spawn()
        │     └─► Background task: health-checks homeserver every 10s
        │     └─► Exits process if homeserver becomes unresponsive
        │
        ├─► SqlDb::connect(database_url)
        │     └─► Creates PgPool connection
        │     └─► Runs all migrations
        │
        ├─► sms_verification::router(config, db)
        │     └─► Creates module's AppState
        │     └─► Returns configured Router
        │
        ├─► ln_verification::router(config, db)
        │     └─► Creates module's AppState
        │     └─► Spawns InvoiceBackgroundSyncer task
        │     └─► Returns configured Router
        │
        └─► Combines routers, starts HTTP server
```

## Resource Sharing

### Database (SqlDb)

The database is **truly shared** across modules:

```rust
// main.rs creates single SqlDb instance
let db = SqlDb::connect(&config.database_url).await?;

// Both modules receive the same instance (clone is cheap - ref-counted)
let sms_router = sms_verification::router(&config, &db).await?;
let ln_router = ln_verification::router(&config, &db).await?;
```

`SqlDb` wraps `sqlx::PgPool` which is internally reference-counted. Cloning is cheap.

### Configuration (EnvConfig)

Single config instance loaded once, passed by reference:

```rust
let config = EnvConfig::load();  // Single load
sms_verification::router(&config, &db)  // Passed as &EnvConfig
ln_verification::router(&config, &db)   // Same reference
```

### Homeserver Admin API

Each module **instantiates its own client** from shared config:

```rust
// SMS module creates its own instance
let homeserver_api = HomeserverAdminAPI::new(
    &config.homeserver_admin_api_url,
    &config.homeserver_admin_password,
    &config.homeserver_pubky,
);

// LN module creates its own instance (same config values)
let homeserver_api = HomeserverAdminAPI::new(
    &config.homeserver_admin_api_url,
    &config.homeserver_admin_password,
    &config.homeserver_pubky,
);
```

This is safe because `reqwest::Client` (used internally) is ref-counted. Separate instances allow concurrent calls without blocking.

## Directory Structure

```
src/
├── main.rs                      # Entry point, bootstrap
│
├── infrastructure/              # Shared infrastructure layer
│   ├── config.rs                # EnvConfig - environment configuration
│   ├── http/                    # HTTP server, middleware, extractors
│   │   ├── mod.rs
│   │   ├── server.rs            # HttpServer::start(), router composition
│   │   ├── error.rs             # HTTP error types
│   │   └── extractors.rs        # Custom Axum extractors
│   └── sql/                     # Database abstraction
│       ├── mod.rs
│       ├── sql_db.rs            # SqlDb wrapper around PgPool
│       ├── unified_executor.rs  # Abstraction for pool/transaction
│       └── migrations/          # SQL migration files
│
├── shared/                      # Shared services layer
│   ├── mod.rs
│   ├── homeserver_admin_api.rs  # Homeserver API client
│   └── homeserver_observer.rs   # Background health monitor
│
├── sms_verification/            # SMS verification module
│   ├── mod.rs                   # Public exports
│   ├── http.rs                  # Router factory + handlers
│   ├── app_state.rs             # Module's AppState
│   ├── service.rs               # SmsVerificationService
│   ├── repository.rs            # SmsVerificationRepository
│   ├── types.rs                 # PhoneNumber, Code, entities
│   ├── error.rs                 # SmsVerificationError
│   └── prelude_api.rs           # Prelude SMS provider client
│
└── ln_verification/             # Lightning verification module
    ├── mod.rs                   # Public exports
    ├── http.rs                  # Router factory + handlers
    ├── app_state.rs             # Module's AppState
    ├── service.rs               # LnVerificationService
    ├── repository.rs            # LnVerificationRepository
    ├── types.rs                 # PaymentHash, entities
    ├── error.rs                 # LnVerificationError
    ├── phoenixd_api/            # Phoenixd Lightning node client
    └── invoice_background_syncer.rs  # Background invoice polling
```

## Module Structure

Each module follows the same internal pattern:

### Layer Flow

```
http.rs (Inbound Adapter)
    │
    ▼
app_state.rs (Dependency Container)
    │
    ▼
service.rs (Business Logic)
    │
    ├──► repository.rs (Data Access)
    │         │
    │         ▼
    │    infrastructure/sql (Shared Database)
    │
    └──► *_api.rs (External API Client)
           │
           ▼
      External Service (Prelude, Phoenixd, etc.)
```

### Layer Responsibilities

| Layer | File | Responsibility |
|-------|------|----------------|
| **Inbound Adapter** | `http.rs` | Router factory, HTTP handlers, request/response mapping |
| **App State** | `app_state.rs` | Dependency wiring, holds service references |
| **Service** | `service.rs` | Business logic, orchestration, use case implementation |
| **Domain** | `types.rs` | Value objects with validation, entities |
| **Repository** | `repository.rs` | Database queries via UnifiedExecutor |
| **External API** | `*_api.rs` | HTTP client for external services |
| **Error** | `error.rs` | Module-specific error types with conversions |

### Router Factory Pattern

Each module exposes a `router()` function that:
1. Receives shared infrastructure (config, db)
2. Creates module-specific dependencies internally
3. Wires up AppState
4. Returns a configured Router with state embedded

```rust
// sms_verification/http.rs
pub async fn router(
    config: &EnvConfig,
    db: &SqlDb,
) -> Result<Router, HttpServerError> {
    let state = AppState::new(config, db.clone());

    Ok(Router::new()
        .route("/", post(create_verification))
        .route("/validate", post(validate_code))
        .with_state(state))
}
```

The main HTTP server nests module routers:

```rust
// infrastructure/http/server.rs
Router::new()
    .route("/", get(health_check))
    .nest("/sms", sms_router)
    .nest("/ln", ln_router)
    .layer(TraceLayer::new_for_http())
```

## Dependency Rules

### Acyclic Directed Graph (DAG)

Modules **can depend on other modules** as long as dependencies form a **top-down acyclic graph**. No circular dependencies allowed.

**Example: Verification Modules with Shared Provider**

```
┌────────────────────┐   ┌───────────────────────┐   ┌───────────────────────┐
│                    │   │                       │   │                       │
│  SMS Verification  │   │ Telegram Verification │   │ Lightning Verification│
│                    │   │                       │   │                       │
└─────────┬──────────┘   └───────────┬───────────┘   └───────────┬───────────┘
          │                          │                           │
          │                          │                           │
┌─────────▼──────────────────────────▼───────────┐               │
│                                                │               │
│           Prelude Verifications                │               │
│           (shared provider module)             │               │
│                                                │               │
└────────────────────────┬───────────────────────┘               │
                         │                                       │
┌────────────────────────▼───────────────────────────────────────▼───────────┐
│                                                                            │
│                            Homeserver API                                  │
│                                                                            │
└────────────────────────────────────┬───────────────────────────────────────┘
                                     │
┌────────────────────────────────────▼───────────────────────────────────────┐
│                                                                            │
│                              Database                                      │
│                                                                            │
└────────────────────────────────────┬───────────────────────────────────────┘
                                     │
┌────────────────────────────────────▼───────────────────────────────────────┐
│                                                                            │
│                               Config                                       │
│                                                                            │
└────────────────────────────────────────────────────────────────────────────┘
```

In this example:
- `sms_verification` and `telegram_verification` both depend on `prelude_verifications`
- `prelude_verifications` provides the shared Prelude API integration
- `lightning_verification` is independent, doesn't use Prelude
- All modules depend on shared infrastructure (Homeserver, Database, Config)

### Rules

1. **Dependencies must be acyclic** - If A depends on B, B cannot depend on A (directly or transitively)
2. **Dependencies flow downward** - Higher-level modules depend on lower-level modules
3. **Prefer independence** - Keep modules independent when possible, but allow dependencies when it reduces duplication
4. **Infrastructure has no business logic** - Only technical concerns

### Valid vs Invalid Dependencies

```
✅ VALID (DAG - no cycles):

sms_verification ──► prelude_verifications ──► homeserver
telegram_verification ──► prelude_verifications ──► homeserver
ln_verification ──► homeserver


❌ INVALID (cycle):

module_a ──► module_b
    ▲            │
    └────────────┘
```

### When to Create a Shared Module

Create a shared module (like `prelude_verifications`) when:
- Multiple modules need the same external API integration
- Business logic is duplicated across modules
- A clear abstraction boundary exists

Keep modules separate when:
- They have different external providers (SMS vs Lightning)
- Coupling would create unnecessary complexity
- Independence aids testing and deployment

## Background Tasks

### HomeserverObserver

Spawned at startup, runs continuously:
- Health-checks homeserver every 10 seconds
- **Exits entire process** if homeserver becomes unresponsive
- Homegate cannot function without homeserver

### InvoiceBackgroundSyncer (LN Module)

Spawned by ln_verification module:
- Polls Phoenixd for payment status changes
- Updates database when payments are received
- Uses broadcast channel to notify waiting HTTP handlers

```rust
// ln_verification/http.rs
let syncer = InvoiceBackgroundSyncer::new(ln_service.clone(), phoenixd_api).await;
tokio::task::spawn(async move {
    syncer.clone().run().await;
});
```

## Error Handling

Each module defines its own error type with conversions:

```rust
// ln_verification/error.rs
#[derive(Debug, thiserror::Error)]
pub enum LnVerificationError {
    #[error("Invoice not found")]
    InvoiceNotFound,

    #[error("Phoenixd error: {0}")]
    Phoenixd(#[from] PhoenixdError),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// Converts to HTTP response
impl IntoResponse for LnVerificationError { ... }
```

Error flow: `ExternalAPIError → ModuleError → HTTP Response`

## Testing Strategy

| Test Type | Scope | Tools |
|-----------|-------|-------|
| **Unit** | Service logic | Mock external APIs |
| **Integration** | Repository + DB | `#[sqlx::test]` with real Postgres |
| **E2E** | Full HTTP flow | Wiremock for external APIs |

## Checklist for New Modules

When adding a new feature module:

- [ ] Create module directory under `src/`
- [ ] Add `mod.rs` with public exports
- [ ] Create `types.rs` with domain types and entities
- [ ] Create `error.rs` with module-specific errors
- [ ] Create `app_state.rs` with AppState struct
- [ ] Create `service.rs` with business logic
- [ ] Create `repository.rs` if persisting data
- [ ] Create `http.rs` with router factory and handlers
- [ ] Create `*_api.rs` for each external API integration
- [ ] Add `pub mod new_module;` to `main.rs`
- [ ] Call `new_module::router()` in `HttpServer::start()`
- [ ] Nest router under appropriate path
- [ ] Add database migrations if needed
- [ ] Write tests
