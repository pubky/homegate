# Modular Hexagonal Architecture

A pragmatic architecture pattern combining **Feature Modules** with **Hexagonal (Ports & Adapters)** principles. Optimized for maintainability, testability, and clarity without excessive abstraction.

## Core Principles

1. **Feature-first organization** - Code is organized by business capability, not technical layer
2. **Hexagonal boundaries** - External systems are abstracted behind adapters
3. **Inward dependencies** - Dependencies point toward the domain, never outward
4. **Pragmatic abstractions** - Abstract where it adds value, not for purity
5. **Module independence** - Modules avoid direct dependencies on each other

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                      APPLICATION                                │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐  │
│  │  Feature Module │  │  Feature Module │  │  Feature Module │  │
│  │  ┌───────────┐  │  │  ┌───────────┐  │  │  ┌───────────┐  │  │
│  │  │  Inbound  │  │  │  │  Inbound  │  │  │  │  Inbound  │  │  │
│  │  │  Adapter  │  │  │  │  Adapter  │  │  │  │  Adapter  │  │  │
│  │  └─────┬─────┘  │  │  └─────┬─────┘  │  │  └─────┬─────┘  │  │
│  │        ▼        │  │        ▼        │  │        ▼        │  │
│  │  ┌───────────┐  │  │  ┌───────────┐  │  │  ┌───────────┐  │  │
│  │  │  Service  │  │  │  │  Service  │  │  │  │  Service  │  │  │
│  │  │  (Logic)  │  │  │  │  (Logic)  │  │  │  │  (Logic)  │  │  │
│  │  └─────┬─────┘  │  │  └─────┬─────┘  │  │  └─────┬─────┘  │  │
│  │        ▼        │  │        ▼        │  │        ▼        │  │
│  │  ┌───────────┐  │  │  ┌───────────┐  │  │  ┌───────────┐  │  │
│  │  │ Outbound  │  │  │  │ Outbound  │  │  │  │ Outbound  │  │  │
│  │  │ Adapters  │  │  │  │ Adapters  │  │  │  │ Adapters  │  │  │
│  │  └───────────┘  │  │  └───────────┘  │  │  └───────────┘  │  │
│  └─────────────────┘  └─────────────────┘  └─────────────────┘  │
│                                                                 │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                 SHARED INFRASTRUCTURE                   │    │
│  │  Config │ Database │ HTTP Server │ Shared Services      │    │
│  └─────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────┘
```

## Directory Structure

```
src/
├── main.rs                      # Entry point, bootstrap
│
├── infrastructure/              # Shared infrastructure
│   ├── config.rs                # Environment/configuration
│   ├── http/                    # HTTP server, middleware, extractors
│   │   ├── mod.rs
│   │   ├── server.rs
│   │   ├── error.rs
│   │   └── extractors.rs
│   └── database/                # Database abstraction
│       ├── mod.rs
│       ├── connection.rs
│       └── migrations/
│
├── shared/                      # Shared services (used by multiple modules)
│   ├── mod.rs
│   └── external_api.rs          # Shared external API clients
│
├── <feature_module>/            # Feature module (repeat for each feature)
│   ├── mod.rs                   # Module exports
│   ├── http.rs                  # Inbound adapter: router factory + handlers
│   ├── app_state.rs             # Module's AppState and dependency wiring
│   ├── service.rs               # Application logic
│   ├── repository.rs            # Outbound adapter: data access
│   ├── types.rs                 # Domain types, value objects
│   ├── error.rs                 # Module-specific errors
│   └── <external>_api.rs        # Outbound adapter: external API client

```

## Feature Module Structure

Each feature module is a vertical slice containing all layers for one business capability.

### Layer Responsibilities

| Layer | File | Responsibility |
|-------|------|----------------|
| **Inbound Adapter** | `http.rs` | Router factory, HTTP handlers, delegates to service |
| **App State** | `app_state.rs` | Dependency wiring, holds service and db references |
| **Service** | `service.rs` | Business logic, orchestration, use case implementation |
| **Domain** | `types.rs` | Value objects, domain types with validation |
| **Outbound Adapter** | `repository.rs` | Data persistence, database queries |
| **Outbound Adapter** | `*_api.rs` | External API integration |
| **Error** | `error.rs` | Module-specific error types |

### Example Module: `user_verification/`

```
user_verification/
├── mod.rs              # pub use exports
├── http.rs             # router() + POST /verify, GET /status handlers
├── app_state.rs        # AppState with db, service
├── service.rs          # UserVerificationService
├── repository.rs       # UserVerificationRepository
├── types.rs            # UserId, VerificationCode, VerificationStatus
├── error.rs            # UserVerificationError
└── provider_api.rs     # External verification provider client
```

## Layer Details

### Inbound Adapter (http.rs)

Handles HTTP concerns only. Each module provides a router factory function that:
1. Creates the module's AppState internally
2. Builds the router with routes
3. Embeds state via `.with_state()`
4. Returns a plain `Router` (state already attached)

```rust
pub async fn router(
    config: &EnvConfig,
    db: &SqlDb,
) -> Result<Router, HttpServerError> {
    // Module creates its own state
    let state = AppState::new(config, db.clone());

    Ok(Router::new()
        .route("/", post(create_handler))
        .route("/:id", get(get_handler))
        .with_state(state))
}

async fn create_handler(
    State(state): State<AppState>,
    Json(request): Json<CreateRequest>,
) -> Result<Json<Response>, HttpError> {
    let result = state.service.create(request).await?;
    Ok(Json(result))
}
```

The main HTTP server then nests each module's router under its path:

```rust
// infrastructure/http/server.rs
pub fn create_router(
    user_verification_router: Router,
    payment_router: Router,
) -> Router {
    Router::new()
        .route("/", get(health_check))
        .nest("/user_verification", user_verification_router)
        .nest("/payments", payment_router)
        .layer(TraceLayer::new_for_http())
}
```

### Service (service.rs)

Contains business logic. Orchestrates repositories and external APIs.

```rust
pub struct UserVerificationService {
    db: SqlDb,
    provider_api: ProviderApi,
    external_service: ExternalService,
}

impl UserVerificationService {
    pub async fn create(&self, request: CreateRequest) -> Result<Response, Error> {
        // 1. Validate business rules
        // 2. Call external API if needed
        // 3. Persist via repository
        // 4. Return result
    }
}
```

### Domain Types (types.rs)

Value objects with validation. No dependencies on other layers.

```rust
pub struct VerificationCode(String);

impl VerificationCode {
    pub fn new(value: &str) -> Result<Self, ValidationError> {
        if value.len() != 6 || !value.chars().all(|c| c.is_ascii_digit()) {
            return Err(ValidationError::InvalidCode);
        }
        Ok(Self(value.to_string()))
    }
}

// Request/Response types
pub struct CreateRequest {
    pub user_id: UserId,
    pub method: VerificationMethod,
}
```

### Repository (repository.rs)

Data access with static methods. Pragmatic approach without trait abstraction.

```rust
pub struct UserVerificationRepository;

impl UserVerificationRepository {
    pub async fn create(
        executor: impl Executor<'_, Database = Postgres>,
        entity: &VerificationEntity,
    ) -> Result<i64, RepositoryError> {
        // SQL query using query builder or raw SQL
    }

    pub async fn find_by_id(
        executor: impl Executor<'_, Database = Postgres>,
        id: i64,
    ) -> Result<Option<VerificationEntity>, RepositoryError> {
        // ...
    }

    pub async fn update_status(
        executor: impl Executor<'_, Database = Postgres>,
        id: i64,
        status: VerificationStatus,
    ) -> Result<(), RepositoryError> {
        // ...
    }
}
```

### External API Adapter (*_api.rs)

Wraps external HTTP APIs. Handles authentication, serialization, error mapping.

```rust
pub struct ProviderApi {
    http_client: reqwest::Client,
    base_url: Url,
    api_key: String,
}

impl ProviderApi {
    pub async fn send_code(&self, phone: &str) -> Result<ProviderResponse, ProviderError> {
        let response = self.http_client
            .post(self.base_url.join("/send")?)
            .bearer_auth(&self.api_key)
            .json(&request)
            .send()
            .await?;
        // Handle response, map errors
    }
}
```

### Error Types (error.rs)

Module-specific errors with conversions for clean propagation.

```rust
#[derive(Debug, thiserror::Error)]
pub enum UserVerificationError {
    #[error("User not found")]
    UserNotFound,

    #[error("Code expired")]
    CodeExpired,

    #[error("Provider error: {0}")]
    Provider(#[from] ProviderError),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// Convert to HTTP response
impl IntoResponse for UserVerificationError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::UserNotFound => StatusCode::NOT_FOUND,
            Self::CodeExpired => StatusCode::GONE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}
```

## Dependency Rules

### Allowed Dependencies

```
http.rs ──────► service.rs ──────► repository.rs
                    │                    │
                    │                    ▼
                    │              infrastructure/database
                    │
                    ├──────► types.rs (domain)
                    │
                    ├──────► *_api.rs (external APIs)
                    │
                    └──────► shared/ (shared services)
```

### Forbidden Dependencies

- Modules should NOT import from other feature modules
- Lower layers should NOT import from upper layers
- Domain types should NOT import from any other layer

## Shared Infrastructure

### What belongs in `infrastructure/`

- Database connection, pooling, migrations
- HTTP server setup, middleware, common extractors
- Configuration loading
- Logging/tracing setup

### What belongs in `shared/`

- Services used by multiple modules (e.g., notification service, admin API)
- Shared external API clients
- Common utilities that aren't infrastructure

## App State & Dependency Injection

Each module defines its own `AppState` struct. State is created within the module's router function, keeping dependency wiring contained within the module.

```rust
// feature_module/app_state.rs
#[derive(Clone)]
pub struct AppState {
    pub db: SqlDb,
    pub service: FeatureService,
}

impl AppState {
    pub fn new(config: &EnvConfig, db: SqlDb) -> Self {
        let provider_api = ProviderApi::new(&config.provider_url, &config.provider_key);
        let external_service = ExternalService::new(&config.external_url);
        let service = FeatureService::new(db.clone(), provider_api, external_service);

        Self { db, service }
    }
}
```

```rust
// feature_module/http.rs
pub async fn router(config: &EnvConfig, db: &SqlDb) -> Result<Router, Error> {
    let state = AppState::new(config, db.clone());
    Ok(Router::new()
        .route("/", post(handler))
        .with_state(state))
}
```

This pattern:
- Keeps module dependencies self-contained
- Makes it clear what each module needs
- Allows different modules to have different state shapes

## Module Communication

### Preferred: Shared Services

Modules communicate through shared services, not directly.

```rust
// shared/notification_service.rs
pub struct NotificationService { ... }

// Module A uses it
impl ModuleAService {
    pub fn new(notification: NotificationService) -> Self { ... }
}

// Module B uses it
impl ModuleBService {
    pub fn new(notification: NotificationService) -> Self { ... }
}
```

### Avoid: Direct Module Dependencies

```rust
// ❌ Don't do this
use crate::other_module::OtherService;

impl MyService {
    pub fn do_something(&self) {
        self.other_service.call();  // Direct coupling
    }
}
```

### When Modules Need to Interact

If modules genuinely need to interact:
1. Extract shared logic to `shared/`
2. Use events/messages for loose coupling (if complexity warrants it)
3. Accept that some coupling may be appropriate for your use case

## Testing Strategy

### Unit Tests

Test services with mocked dependencies (external APIs).

```rust
#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn test_create_verification() {
        let mock_api = MockProviderApi::new();
        let service = Service::new(mock_api);

        let result = service.create(request).await;
        assert!(result.is_ok());
    }
}
```

### Integration Tests

Test repositories against real database.

```rust
#[sqlx::test]
async fn test_repository_create(pool: PgPool) {
    let result = Repository::create(&pool, &entity).await;
    assert!(result.is_ok());
}
```

### E2E Tests

Test full HTTP flow with mocked external APIs (Wiremock).

```rust
#[tokio::test]
async fn test_full_verification_flow() {
    let mock_server = MockServer::start().await;
    // Setup mock responses
    // Make HTTP requests to your server
    // Assert responses
}
```

## When to Use This Architecture

**Good fit:**
- Medium to large applications with multiple business capabilities
- Teams that want clear boundaries without microservice overhead
- Applications integrating with multiple external systems
- Projects valuing testability and maintainability

**Consider alternatives when:**
- Very small applications (single feature) - simpler structure may suffice
- Strict microservice requirements - extract modules to separate services
- Heavy domain logic - consider richer Domain-Driven Design patterns

## Trade-offs

| Benefit | Cost |
|---------|------|
| Clear module boundaries | More files per feature |
| Easy to test in isolation | Initial setup overhead |
| External systems abstracted | Must maintain adapter layer |
| Easy to find related code | Some duplication across modules |
| Could extract to microservices | Not zero-cost extraction |

## Checklist for New Modules

When creating a new feature module:

- [ ] Create module directory under `src/`
- [ ] Add `mod.rs` with public exports
- [ ] Create `types.rs` with domain types
- [ ] Create `error.rs` with module errors
- [ ] Create `app_state.rs` with module's AppState
- [ ] Create `service.rs` with business logic
- [ ] Create `repository.rs` if persisting data
- [ ] Create `http.rs` with router factory and handlers
- [ ] Create `*_api.rs` for each external API
- [ ] Nest module router in main HTTP server (`server.rs`)
- [ ] Add database migrations if needed
- [ ] Write tests
