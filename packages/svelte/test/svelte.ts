import { plugin } from "bun";
import { compile } from "svelte/compiler";

// Compile the `.svelte` files the tests import, as Vite's Svelte plugin
// does in an editor. Imported before any of them.
plugin({
  name: "svelte",
  setup(build) {
    build.onLoad({ filter: /\.svelte$/ }, async ({ path }) => {
      const source = await Bun.file(path).text();
      const { js, warnings } = compile(source, { filename: path, generate: "client" });
      for (const warning of warnings) console.warn(`${path}: ${warning.message}`);
      return { contents: js.code, loader: "js" };
    });
  },
});
