(function() {
  var paramMap = {};
  var paramById = {};
  var pendingParamSubs = {};
  var eventListeners = {};
  var invokeCallbacks = {};
  var nextCallId = 0;
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

  window.__BEAMER__ = {
    ready: readyPromise,

    params: {
      get: function(stringId) {
        var p = paramMap[stringId];
        return p ? p.value : 0;
      },
      set: function(stringId, value) {
        var p = paramMap[stringId];
        if (!p) return;
        p.value = value;
        p.info.value = value;
        post({type:"param:set", id:p.id, value:value});
      },
      beginEdit: function(stringId) {
        var p = paramMap[stringId];
        if (p) post({type:"param:begin", id:p.id});
      },
      endEdit: function(stringId) {
        var p = paramMap[stringId];
        if (p) post({type:"param:end", id:p.id});
      },
      on: function(stringId, cb) {
        var p = paramMap[stringId];
        if (!p) {
          if (!pendingParamSubs[stringId]) pendingParamSubs[stringId] = [];
          pendingParamSubs[stringId].push(cb);
          return function() {
            var arr = pendingParamSubs[stringId];
            if (arr) {
              pendingParamSubs[stringId] = arr.filter(function(f){return f!==cb;});
              return;
            }
            var q = paramMap[stringId];
            if (q) q.listeners = q.listeners.filter(function(f){return f!==cb;});
          };
        }
        p.listeners.push(cb);
        return function() {
          p.listeners = p.listeners.filter(function(f){return f!==cb;});
        };
      },
      all: function() {
        return Object.values(paramMap).map(function(p) { return p.info; });
      },
      info: function(stringId) {
        var p = paramMap[stringId];
        return p ? p.info : undefined;
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

    _onInit: function(params) {
      params.forEach(function(p) {
        var pending = pendingParamSubs[p.stringId] || [];
        delete pendingParamSubs[p.stringId];
        var entry = {
          id: p.id, value: p.value, listeners: pending,
          info: p
        };
        paramMap[p.stringId] = entry;
        paramById[p.id] = entry;
      });
      readyResolve();
    },

    _onParams: function(changed) {
      for (var id in changed) {
        var entry = paramById[id];
        if (entry) {
          entry.value = changed[id];
          entry.info.value = changed[id];
          entry.listeners.forEach(function(cb) { cb(entry.value); });
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
      if (cbs) cbs.forEach(function(cb) { cb(data); });
    }
  };
})();
