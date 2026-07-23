import init, * as wasm from "./pkg/index.js";

const ready = init().then(() => {
  api.version = wasm.version();
  api.drafts = wasm.drafts();
});

const api = {
  ready,
  validate: (schema, instance, options) => ready.then(() => wasm.validate(schema, instance, options)),
  bundle: (schema, options) => ready.then(() => wasm.bundle(schema, options)),
  dereference: (schema, options) => ready.then(() => wasm.dereference(schema, options)),
  canonicalize: (schema, options) => ready.then(() => wasm.canonicalize(schema, options)),
  metaValidate: (schema, options) => ready.then(() => wasm.meta_validate(schema, options)),
};

window.JSONSchema = api;
