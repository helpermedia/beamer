// Shared IPC helpers for AU codegen templates.
//
// Included by auv2_wrapper.c, auv3_wrapper.m and auv3_extension_gui.m.
// These static functions handle invoke, event and init-dump dispatch
// that is identical across all AU format variants.

#pragma once

#include "BeamerAuBridge.h"

// ---------------------------------------------------------------------------
// Parameter set echo
// ---------------------------------------------------------------------------

/// Echo authoritative parameter values back to JS after a param:set.
///
/// Called immediately after updating the Rust store so displayText
/// updates without waiting for the next poll tick. Also updates the
/// poll cache to prevent redundant re-sends.
static void beamer_au_ipc_echo_param(
    BeamerAuInstanceHandle instance,
    void* webviewHandle,
    uint32_t paramId,
    double* lastParamValues,
    uint32_t paramCount
) {
    if (!instance || !webviewHandle) return;

    double norm = beamer_au_param_get_normalized(instance, paramId);
    double plain = beamer_au_param_get_plain(instance, paramId);
    char text[128];
    beamer_au_param_get_display_text(instance, paramId, text, sizeof(text));

    // Escape the text for embedding in a JS string literal.
    NSString* textStr = [NSString stringWithUTF8String:text];
    textStr = [textStr stringByReplacingOccurrencesOfString:@"\\" withString:@"\\\\"];
    textStr = [textStr stringByReplacingOccurrencesOfString:@"\"" withString:@"\\\""];
    textStr = [textStr stringByReplacingOccurrencesOfString:@"\n" withString:@"\\n"];
    textStr = [textStr stringByReplacingOccurrencesOfString:@"\r" withString:@"\\r"];

    NSString* script = [NSString stringWithFormat:
        @"window.__BEAMER__._onParams({%u:[%.17g,%.17g,\"%@\"]})",
        paramId, norm, plain, textStr];
    const char* utf8 = [script UTF8String];
    beamer_webview_eval_js(webviewHandle, (const uint8_t*)utf8, strlen(utf8));

    // Update poll cache so the next tick doesn't redundantly re-send.
    if (lastParamValues) {
        BeamerAuParameterInfo info;
        for (uint32_t i = 0; i < paramCount; i++) {
            if (beamer_au_get_parameter_info(instance, i, &info) && info.id == paramId) {
                lastParamValues[i] = norm;
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Built-in invoke dispatch
// ---------------------------------------------------------------------------

/// Handle built-in Beamer invokes (prefixed with "_beamer/").
///
/// Returns YES if the method was handled, NO if the caller should
/// fall through to the plugin handler.
static BOOL beamer_au_ipc_handle_builtin_invoke(
    BeamerAuInstanceHandle instance,
    void* webviewHandle,
    NSDictionary* msg
) {
    NSString* method = msg[@"method"];
    if (![method hasPrefix:@"_beamer/"]) return NO;

    NSNumber* callId = msg[@"callId"];
    if (!callId) return YES;

    if ([method isEqualToString:@"_beamer/paramTextToNormalized"]) {
        NSArray* args = msg[@"args"];
        uint32_t paramId = [args[0] unsignedIntValue];
        NSString* text = args[1];
        const char* textUtf8 = [text UTF8String];
        size_t textLen = strlen(textUtf8);

        double normalized = beamer_au_param_string_to_normalized(
            instance, paramId, (const uint8_t*)textUtf8, textLen);

        NSString* script;
        if (isnan(normalized)) {
            script = [NSString stringWithFormat:
                @"window.__BEAMER__._onResult(%@,{\"ok\":null})", callId];
        } else {
            script = [NSString stringWithFormat:
                @"window.__BEAMER__._onResult(%@,{\"ok\":%.17g})", callId, normalized];
        }
        const char* utf8 = [script UTF8String];
        beamer_webview_eval_js(webviewHandle, (const uint8_t*)utf8, strlen(utf8));
        return YES;
    }

    return NO;
}

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
