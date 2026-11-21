// The ids and value types of the plugin's model, as the page knows them.
//
// A page with a generated `model-types.ts` (written from the model's Rust
// code) augments `ModelTypes`. Each name the functions of these packages
// take is then one the model declares. Each value has the model's type. A
// section the file does not hold stays loose: any name, values `unknown`.
// The `events` and `streams` sections share their names: a file that holds
// one of the two gets the other as empty. Without the file every section
// is loose.

/**
 * What the model's generated file tells of it, through module
 * augmentation:
 *
 *     declare module "@helpermedia/beamer-api" {
 *       interface ModelTypes {
 *         parameters: "gain" | "pan";
 *         state: { pan_law: PanLaw };
 *         values: { levels: Levels };
 *         streams: { history: number };
 *         events: { sampleRate: number };
 *         pageEvents: {};
 *         methods: { info: { arguments: []; result: Info } };
 *       }
 *     }
 *
 * Empty here. An empty section in the file (`{}`, `never` for the
 * parameters) allows no name, apart from the built-in method.
 * The file must be a module (it imports or exports something): a
 * `declare module` block in a script replaces beamer-api's types instead
 * of adding to them.
 */
export interface ModelTypes {}

// The section `K` of the model's types, or `Loose` while the file holds
// none.
type Section<K extends string, Loose> = K extends keyof ModelTypes ? ModelTypes[K] : Loose;

// Any name with a value of unknown type.
type AnyName = { [name: string]: unknown };

/** The string id of a parameter: its `id` behind the names of the nested groups it sits in. */
export type ParameterId = Extract<Section<"parameters", string>, string>;

type StateTypes = Section<"state", AnyName>;

/** The path of a `State` field. */
export type StatePath = keyof StateTypes & string;

/** The value of the `State` field at `P`. */
export type StateValue<P extends StatePath> = StateTypes[P];

type ValueTypes = Section<"values", AnyName>;

/** The name of a value sender. */
export type ValueSenderName = keyof ValueTypes & string;

/** The value the value sender `N` sends. */
export type SenderValue<N extends ValueSenderName> = ValueTypes[N];

// Stream senders arrive as events: an array of what was pushed since the
// last tick. Events and streams share their names, so they are loose
// together: while the file holds neither section.
// A name in both sections, which only a hand-written declaration can have,
// carries either data.
type EventTypes = Section<"events", {}>;
type StreamTypes = Section<"streams", {}>;
type PluginEventTypes = Extract<keyof ModelTypes, "events" | "streams"> extends never
  ? AnyName
  : {
      [N in keyof EventTypes | keyof StreamTypes]:
        | (N extends keyof EventTypes ? EventTypes[N] : never)
        | (N extends keyof StreamTypes ? StreamTypes[N][] : never);
    };

/** The name of an event the plugin emits or of a stream sender. */
export type PluginEventName = keyof PluginEventTypes & string;

/** The data of the event `N`: an array of values for a stream sender. */
export type PluginEventData<N extends PluginEventName> = PluginEventTypes[N];

type PageEventTypes = Section<"pageEvents", AnyName>;

/** The name of an event the page emits to the plugin. */
export type PageEventName = keyof PageEventTypes & string;

/**
 * The data that goes with the page event `N`, as the arguments of `emit`
 * after the name: none or `null` for an event without data, optional
 * while the page's events are loose.
 */
export type PageEventArguments<N extends PageEventName> = "pageEvents" extends keyof ModelTypes
  ? PageEventTypes[N] extends null
    ? [data?: null]
    : [data: PageEventTypes[N]]
  : [data?: unknown];

// The methods the runtime answers itself, with or without the file.
type BuiltInMethods = {
  "_beamer/parameterTextToNormalized": { arguments: [id: number, text: string]; result: number | null };
};

type MethodTypes = Section<"methods", { [name: string]: { arguments: unknown[]; result: unknown } }> & BuiltInMethods;

/** The name of a method the plugin answers through `invoke`. */
export type MethodName = keyof MethodTypes & string;

/** The arguments of the method `M`, in order. */
export type MethodArguments<M extends MethodName> = MethodTypes[M]["arguments"];

/** What the method `M` resolves to. */
export type MethodResult<M extends MethodName> = MethodTypes[M]["result"];
