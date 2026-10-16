// Shared IPC helpers for AU codegen templates.
//
// Included by auv2_wrapper.c, auv3_wrapper.m and auv3_extension_gui.m.
// These static functions keep the web UI in step (through the editor
// sync of the bridge) and dispatch invokes and events the same way in all
// AU format variants.

#pragma once

#include "BeamerAuBridge.h"

// ---------------------------------------------------------------------------
// Editor sync
// ---------------------------------------------------------------------------

/// Evaluate a script that Rust built in the WebView, then free it.
static void beamer_au_ipc_eval_script(void* webviewHandle, char* script) {
    if (script && webviewHandle) {
        beamer_webview_eval_js(webviewHandle, (const uint8_t*)script, strlen(script));
    }
    beamer_au_free_string(script);
}

/// Answer a param:set with the value in effect.
///
/// Called after the Rust store was set and the host was told, so that the
/// display text follows at once and the next frame does not send the value
/// again.
static void beamer_au_ipc_answer_param(
    BeamerAuEditorSyncHandle sync,
    void* webviewHandle,
    uint32_t paramId
) {
    if (!sync || !webviewHandle) return;
    beamer_au_ipc_eval_script(webviewHandle, beamer_au_editor_sync_parameter_script(sync, paramId));
}

/// Handle a "state:set" message: set the State field in Rust and answer
/// with the value in effect. Takes the message as the web UI sent it.
static void beamer_au_ipc_handle_state_set(
    BeamerAuEditorSyncHandle sync,
    void* webviewHandle,
    const uint8_t* json,
    size_t len
) {
    if (!sync) return;
    beamer_au_ipc_eval_script(webviewHandle, beamer_au_editor_sync_message(sync, json, len));
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
// Init dump and frames
// ---------------------------------------------------------------------------

/// Send the parameter init dump to the WebView.
///
/// Called when the WebView finishes loading, with the plugin format the
/// editor runs in ("auv2" or "auv3").
static void beamer_au_ipc_send_init_dump(
    BeamerAuEditorSyncHandle sync,
    void* webviewHandle,
    const char* pluginFormat
) {
    if (!sync || !webviewHandle) return;
    beamer_au_ipc_eval_script(webviewHandle, beamer_au_editor_sync_init_script(sync, pluginFormat));
}

/// Send one frame to the WebView: the parameters that changed and the
/// events the plugin emitted since the last frame.
///
/// Called from the sync timer of an editor.
static void beamer_au_ipc_send_frame(BeamerAuEditorSyncHandle sync, void* webviewHandle) {
    if (!sync || !webviewHandle) return;
    beamer_au_ipc_eval_script(webviewHandle, beamer_au_editor_sync_frame_script(sync));
}

/// Stop keeping an editor that goes away in step and clear its handle.
static void beamer_au_ipc_close_sync(BeamerAuEditorSyncHandle* sync) {
    if (*sync == NULL) return;

    beamer_au_editor_sync_close(*sync);
    *sync = NULL;
}
