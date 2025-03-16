use referencing::Resolver;

#[derive(Clone)]
pub(crate) struct CompilationContext<'r> {
    resolver: Resolver<'r>,
}

impl<'r> CompilationContext<'r> {
    pub(crate) fn new(resolver: Resolver<'r>) -> Self {
        Self { resolver }
    }

    pub(crate) fn resolver(&self) -> &Resolver<'r> {
        &self.resolver
    }
}
