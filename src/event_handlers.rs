use std::future::Future;

pub enum OnDisconnectHandler {
    None,
    Sync(Box<dyn FnOnce() + Send>),
    Async(Box<dyn FnOnce() -> Box<dyn Future<Output = ()> + Send + Unpin> + Send>),
}
impl OnDisconnectHandler {
    pub(crate) async fn run(self) {
        match self {
            OnDisconnectHandler::None => {}
            OnDisconnectHandler::Sync(f) => f(),
            OnDisconnectHandler::Async(f) => f().await,
        }
    }

    #[must_use]
    pub fn take(&mut self) -> Self {
        std::mem::replace(self, OnDisconnectHandler::None)
    }

    pub fn from_async<F, FUTURE>(func: F) -> Self
    where
        F: FnOnce() -> FUTURE + Send + 'static,
        FUTURE: Future<Output = ()> + Send + 'static,
    {
        OnDisconnectHandler::Async(Box::new(move || Box::new(Box::pin(func()))))
    }

    pub fn from_sync<F: FnOnce() + Send + 'static>(func: F) -> Self {
        OnDisconnectHandler::Sync(Box::new(func))
    }
}

#[allow(clippy::type_complexity)]
pub enum SubscriptionHandler {
    Sync(Box<dyn Fn(Vec<u8>) + Send + Sync>),
    Async(Box<dyn Fn(Vec<u8>) -> Box<dyn Future<Output = ()> + Send + Unpin> + Send + Sync>),
}

impl SubscriptionHandler {
    pub fn from_async<F, FUTURE>(func: F) -> Self
    where
        F: Fn(Vec<u8>) -> FUTURE + Send + Sync + 'static,
        FUTURE: Future<Output = ()> + Send + 'static,
    {
        SubscriptionHandler::Async(Box::new(move |data| Box::new(Box::pin(func(data)))))
    }

    pub(crate) async fn run(&self, data: Vec<u8>) {
        match self {
            SubscriptionHandler::Sync(f) => f(data),
            SubscriptionHandler::Async(f) => f(data).await,
        }
    }
}

impl<F: Fn(Vec<u8>) + Send + Sync + 'static> From<F> for SubscriptionHandler {
    fn from(func: F) -> Self {
        SubscriptionHandler::Sync(Box::new(func))
    }
}
