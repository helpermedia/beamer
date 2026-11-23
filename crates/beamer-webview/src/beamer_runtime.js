(function() {
  // The maps by name, id or path have no prototype, so that a name such as
  // "constructor" or "toString" is a name like any other.
  var parameterMap = Object.create(null);
  var parameterById = Object.create(null);
  var pendingParameterSubs = Object.create(null);
  // State fields by path: {value, json, listeners}. `json` is the value as
  // canonical text, to tell a change from a value the UI has already.
  var stateMap = Object.create(null);
  var pendingStateSubs = Object.create(null);
  var eventListeners = Object.create(null);
  // The last value of each value sender, by name.
  var lastValues = Object.create(null);
  var valueListeners = Object.create(null);
  var invokeCallbacks = Object.create(null);
  var nextCallId = 0;
  // The number of the last set the page sent, of any parameter or State
  // field. Each set carries the next one and the plugin's answer carries
  // it back.
  var nextSetSeq = 0;
  // The number of the newest set the page sent, by parameter id and by
  // State path. An answer to an older set than the newest one of the same
  // parameter or field comes too late and is ignored, so that it cannot
  // put an older value back while the newer set is on its way; the answer
  // to the newest set follows. Changes the plugin sends of its own carry
  // no number and always count.
  var sentParameterSeq = Object.create(null);
  var sentStateSeq = Object.create(null);

  // Whether an answer to set `seq` comes after a newer set of the same key.
  function isLate(sent, key, seq) {
    return seq !== undefined && sent[key] !== undefined && seq < sent[key];
  }

  var readyResolve;
  var readyPromise = new Promise(function(r) { readyResolve = r; });

  // JSON.stringify is intentional: postMessage accepts any plist-compatible
  // type, but the native side expects a plain UTF-8 string so it can be
  // forwarded through the C-ABI callback as raw bytes. Passing an object
  // would give an NSDictionary, which is harder to bridge.
  var nativeHandler = window.webkit
    && window.webkit.messageHandlers
    && window.webkit.messageHandlers.beamer;

  function post(msg) {
    if (nativeHandler) nativeHandler.postMessage(JSON.stringify(msg));
  }

  // Post a note for the processor: a note number 0 to 127 and a channel 0
  // to 15 (0 when left out). A velocity left out is the plugin's default;
  // one given is clamped to 0 to 1. A note that does not fit is dropped
  // with a warning.
  function postNote(type, note, velocity, channel) {
    if (channel === undefined) channel = 0;
    var fits = Number.isInteger(note) && note >= 0 && note <= 127
      && Number.isInteger(channel) && channel >= 0 && channel <= 15
      && (velocity === undefined || (typeof velocity === "number" && velocity === velocity));
    if (!fits) {
      console.warn("Beamer: " + type + " needs a note 0 to 127, a channel 0 to 15 and a velocity 0 to 1, not",
        note, channel, velocity);
      return;
    }
    // `type` stays the first key: the AUv3 extension tells a note by the
    // start of the JSON before its audio unit connects.
    var msg = {type:type, note:note, channel:channel};
    if (velocity !== undefined) msg.velocity = Math.min(1, Math.max(0, velocity));
    post(msg);
  }

  // Call a listener of the UI. Native code delivers several changes and
  // events in one script, so a listener that throws must not stop the
  // delivery of what comes after it.
  function notify(cb, value) {
    try {
      cb(value);
    } catch (e) {
      console.error(e);
    }
  }

  // Subscribe to an entry of `entries`, or queue the listener in `pending`
  // until init brings the entry. Returns the function that unsubscribes.
  function subscribe(entries, pending, key, cb) {
    var entry = entries[key];
    if (!entry) {
      if (!pending[key]) pending[key] = [];
      pending[key].push(cb);
      return function() {
        var arr = pending[key];
        if (arr) {
          pending[key] = arr.filter(function(f){return f!==cb;});
          return;
        }
        var e = entries[key];
        if (e) e.listeners = e.listeners.filter(function(f){return f!==cb;});
      };
    }
    entry.listeners.push(cb);
    return function() {
      entry.listeners = entry.listeners.filter(function(f){return f!==cb;});
    };
  }

  // JSON text with the keys of every object sorted, so that two equal
  // values give the same text whatever order their keys came in (a Rust
  // map is written in another order each time).
  function canonical(value) {
    if (value === null || typeof value !== "object") return JSON.stringify(value);
    if (Array.isArray(value)) {
      return "[" + value.map(function(v) {
        var text = canonical(v);
        return text === undefined ? "null" : text;
      }).join(",") + "]";
    }
    var parts = [];
    Object.keys(value).sort().forEach(function(key) {
      var text = canonical(value[key]);
      if (text !== undefined) parts.push(JSON.stringify(key) + ":" + text);
    });
    return "{" + parts.join(",") + "}";
  }

  // Give a State field a new value and tell its listeners, unless the UI
  // has that value already.
  function applyState(path, value) {
    var entry = stateMap[path];
    if (!entry) return;
    var json = canonical(value);
    if (json === entry.json) return;
    entry.value = value;
    entry.json = json;
    entry.listeners.forEach(function(cb) { notify(cb, value); });
  }

  window.__BEAMER__ = {
    ready: readyPromise,

    // The plugin format the editor runs in: "vst3", "auv2" or "auv3". Set
    // before `ready` resolves.
    pluginFormat: null,

    // The name of MIDI note 60 in the plugin's note names: "C3" or "C4"
    // (`middle_c` in Config.toml). Set before `ready` resolves.
    middleC: null,

    parameters: {
      get: function(stringId) {
        var p = parameterMap[stringId];
        return p ? p.value : 0;
      },
      getPlain: function(stringId) {
        var p = parameterMap[stringId];
        return p ? p.plainValue : 0;
      },
      getDisplayText: function(stringId) {
        var p = parameterMap[stringId];
        return p ? p.displayText : "";
      },
      // Set a normalized value. Listeners hear of it at once; the plugin
      // answers with the value in effect and its display text. A value
      // that is not a finite number is refused with a warning: the plugin
      // could not read it and would not answer.
      set: function(stringId, value) {
        var p = parameterMap[stringId];
        if (!p) return;
        if (typeof value !== "number" || !isFinite(value)) {
          console.warn("Beamer: parameter " + stringId + " needs a finite number, not", value);
          return;
        }
        var changed = p.value !== value;
        p.value = value;
        p.info.value = value;
        var seq = ++nextSetSeq;
        sentParameterSeq[p.id] = seq;
        if (changed) p.listeners.forEach(function(cb) { notify(cb, value); });
        post({type:"parameter:set", id:p.id, value:value, seq:seq});
      },
      beginEdit: function(stringId) {
        var p = parameterMap[stringId];
        if (p) post({type:"parameter:begin", id:p.id});
      },
      endEdit: function(stringId) {
        var p = parameterMap[stringId];
        if (p) post({type:"parameter:end", id:p.id});
      },
      on: function(stringId, cb) {
        return subscribe(parameterMap, pendingParameterSubs, stringId, cb);
      },
      all: function() {
        return Object.values(parameterMap).map(function(p) { return p.info; });
      },
      info: function(stringId) {
        var p = parameterMap[stringId];
        return p ? p.info : undefined;
      }
    },

    // The State fields of the plugin, by path ("pan_law", "layer/sample").
    // A value is what JSON makes of the Rust value.
    state: {
      // The value the UI knows: the same object until it changes.
      // Undefined before `ready` and for a path the plugin does not have.
      get: function(path) {
        var entry = stateMap[path];
        return entry ? entry.value : undefined;
      },
      // Set a value. Listeners hear of it at once; the plugin answers with
      // the value in effect, which corrects one that does not fit.
      set: function(path, value) {
        if (!stateMap[path]) return;
        // Keep the value as the plugin will see it: `undefined` becomes
        // null and so does NaN.
        var plain = JSON.parse(JSON.stringify(value === undefined ? null : value));
        applyState(path, plain);
        var seq = ++nextSetSeq;
        sentStateSeq[path] = seq;
        post({type:"state:set", id:path, value:plain, seq:seq});
      },
      on: function(path, cb) {
        return subscribe(stateMap, pendingStateSubs, path, cb);
      },
      // The paths of all State fields. Empty before `ready`.
      ids: function() {
        return Object.keys(stateMap);
      }
    },

    // The values that value senders of the plugin send from the audio
    // thread, by name: the newest of each, as JSON makes it of the Rust
    // value.
    values: {
      // The last value: the same object until a new one arrives.
      // Undefined before the first.
      get: function(name) {
        return lastValues[name];
      },
      // Listen to the values that arrive from now on. The one that
      // arrived already is `get(name)`.
      on: function(name, cb) {
        if (!valueListeners[name]) valueListeners[name] = [];
        valueListeners[name].push(cb);
        return function() {
          valueListeners[name] = valueListeners[name]
            .filter(function(f){return f!==cb;});
        };
      }
    },

    invoke: function(method) {
      var args = Array.prototype.slice.call(arguments, 1);
      return new Promise(function(resolve, reject) {
        var id = nextCallId++;
        invokeCallbacks[id] = {resolve: resolve, reject: reject};
        post({type:"invoke", method:method, args:args, callId:id});
      });
    },

    // Listen to events: what the plugin emits and the values of its
    // stream senders. Not kept: a listener that registers later gets only
    // what comes after.
    on: function(name, cb) {
      if (!eventListeners[name]) eventListeners[name] = [];
      eventListeners[name].push(cb);
      return function() {
        eventListeners[name] = eventListeners[name]
          .filter(function(f){return f!==cb;});
      };
    },

    emit: function(name, data) {
      post({type:"event", name:name, data:data});
    },

    // Notes for the processor, which gets them with the host's MIDI input.
    noteOn: function(note, velocity, channel) {
      postNote("note:on", note, velocity, channel);
    },

    noteOff: function(note, velocity, channel) {
      postNote("note:off", note, velocity, channel);
    },

    allNotesOff: function() {
      // `type` first, as in postNote.
      post({type:"note:all-off"});
    },

    _onInit: function(parameters, info) {
      if (info && info.pluginFormat) {
        window.__BEAMER__.pluginFormat = info.pluginFormat;
      }
      if (info && info.middleC) {
        window.__BEAMER__.middleC = info.middleC;
      }
      parameters.forEach(function(p) {
        var pending = pendingParameterSubs[p.stringId] || [];
        delete pendingParameterSubs[p.stringId];
        var entry = {
          id: p.id, value: p.value, plainValue: p.plainValue,
          displayText: p.displayText, listeners: pending, info: p
        };
        parameterMap[p.stringId] = entry;
        parameterById[p.id] = entry;
        // A listener that subscribed before the value existed hears it now.
        pending.forEach(function(cb) { notify(cb, p.value); });
      });
      var state = (info && info.state) || {};
      Object.keys(state).forEach(function(path) {
        var pending = pendingStateSubs[path] || [];
        delete pendingStateSubs[path];
        stateMap[path] = {
          value: state[path], json: canonical(state[path]), listeners: pending
        };
        pending.forEach(function(cb) { notify(cb, state[path]); });
      });
      readyResolve();
    },

    // `seq` is the number of the set an answer answers, undefined for a
    // change the plugin sends of its own.
    _onState: function(changed, seq) {
      for (var path in changed) {
        if (!isLate(sentStateSeq, path, seq)) applyState(path, changed[path]);
      }
    },

    _onParameters: function(changed, seq) {
      for (var id in changed) {
        var entry = parameterById[id];
        if (entry && !isLate(sentParameterSeq, id, seq)) {
          var tuple = changed[id];
          entry.value = tuple[0];
          entry.plainValue = tuple[1];
          entry.displayText = tuple[2];
          entry.info.value = tuple[0];
          entry.info.plainValue = tuple[1];
          entry.info.displayText = tuple[2];
          entry.listeners.forEach(function(cb) { notify(cb, entry.value); });
        }
      }
    },

    _onResult: function(callId, result) {
      var cb = invokeCallbacks[callId];
      if (!cb) return;
      delete invokeCallbacks[callId];
      if (result && result.err !== undefined) {
        cb.reject(result.err);
      } else {
        cb.resolve(result ? result.ok : null);
      }
    },

    _onEvent: function(name, data) {
      var cbs = eventListeners[name];
      if (cbs) cbs.forEach(function(cb) { notify(cb, data); });
    },

    _onValue: function(name, data) {
      lastValues[name] = data;
      var cbs = valueListeners[name];
      if (cbs) cbs.forEach(function(cb) { notify(cb, data); });
    }
  };
})();
