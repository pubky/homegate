use homegate::{AppContext, HttpServer};

pub struct HomegateService {
    context: AppContext,
    client_server: HttpServer,
}

impl HomegateService {
    pub async fn start(context: AppContext) -> anyhow::Result<Self> {
        let client_server = HttpServer::start(context.clone()).await?;
        Ok(Self {
            context,
            client_server,
        })
    }

    pub fn client_server(&self) -> &HttpServer {
        &self.client_server
    }
}
