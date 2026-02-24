// Shared IPC helpers for AU codegen templates.
//
// Included by auv2_wrapper.c, auv3_wrapper.m and auv3_extension_gui.m.
// These static functions handle invoke, event and init-dump dispatch
// that is identical across all AU format variants.

#pragma once

#include "BeamerAuBridge.h"

// ---------------------------------------------------------------------------
// Invoke dispatch
// ---------------------------------------------------------------------------

/// Handle an "invoke" IPC message.
///
/// Extracts method/args from `msg`, calls `beamer_au_on_invoke` and evals
/// the result back into the WebView so the JS Promise resolves/rejects.
static void beamer_au_ipc_handle_invoke(
    BeamerAuInstanceHandle instance,
    void* webviewHandle,
    NSDictionary* msg
) {
    NSString* method = msg[@"method"];
    NSNumber* callId = msg[@"callId"];
    if (!method || !callId) return;

    NSArray* args = msg[@"args"];
    if (!args) args = @[];
    NSData* argsData = [NSJSONSerialization dataWithJSONObject:args options:0 error:nil];
    if (!argsData) return;
    const uint8_t* argsBytes = (const uint8_t*)[argsData bytes];
    size_t argsLen = [argsData length];

    const char* methodUtf8 = [method UTF8String];
    size_t methodLen = strlen(methodUtf8);

    char* result = beamer_au_on_invoke(instance,
                                       (const uint8_t*)methodUtf8, methodLen,
                                       argsBytes, argsLen);
    if (result && webviewHandle) {
        NSString* script = [NSString stringWithFormat:
            @"window.__BEAMER__._onResult(%@,%s)", callId, result];
        const char* utf8 = [script UTF8String];
        beamer_webview_eval_js(webviewHandle, (const uint8_t*)utf8, strlen(utf8));
    }
    beamer_au_free_string(result);
}

// ---------------------------------------------------------------------------
// Event dispatch
// ---------------------------------------------------------------------------

/// Handle an "event" IPC message.
///
/// Extracts name/data from `msg` and calls `beamer_au_on_event`.
static void beamer_au_ipc_handle_event(
    BeamerAuInstanceHandle instance,
    NSDictionary* msg
) {
    NSString* name = msg[@"name"];
    if (!name) return;

    // Serialize the data value to a JSON string. Wrap in an array to
    // support primitive top-level values, then strip the wrapper.
    id data = msg[@"data"];
    if (!data) data = [NSNull null];
    NSData* dataJson = [NSJSONSerialization dataWithJSONObject:@[data] options:0 error:nil];
    if (!dataJson) return;

    // Strip the array wrapper: "[value]" -> "value"
    NSString* dataStr = [[NSString alloc] initWithData:dataJson encoding:NSUTF8StringEncoding];
    if (!dataStr || dataStr.length < 2) return;
    NSString* unwrapped = [dataStr substringWithRange:NSMakeRange(1, dataStr.length - 2)];
    const char* dataUtf8 = [unwrapped UTF8String];
    size_t dataLen = strlen(dataUtf8);

    const char* nameUtf8 = [name UTF8String];
    size_t nameLen = strlen(nameUtf8);

    beamer_au_on_event(instance,
                       (const uint8_t*)nameUtf8, nameLen,
                       (const uint8_t*)dataUtf8, dataLen);
}

// ---------------------------------------------------------------------------
// Init dump
// ---------------------------------------------------------------------------

/// Send the parameter init dump to the WebView.
///
/// Called when the WebView finishes loading. Serializes all parameter info
/// via `beamer_au_param_info_json` and evals `window.__BEAMER__._onInit(...)`.
static void beamer_au_ipc_send_init_dump(
    BeamerAuInstanceHandle instance,
    void* webviewHandle
) {
    if (!instance || !webviewHandle) return;

    char* json = beamer_au_param_info_json(instance);
    if (!json) return;

    NSString* script = [NSString stringWithFormat:@"window.__BEAMER__._onInit(%s)", json];
    const char* utf8 = [script UTF8String];
    beamer_webview_eval_js(webviewHandle, (const uint8_t*)utf8, strlen(utf8));
    beamer_au_free_string(json);
}
