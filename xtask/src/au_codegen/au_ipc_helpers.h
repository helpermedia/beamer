// Shared IPC helpers for AU codegen templates.
//
// Included by auv2_wrapper.c, auv3_wrapper.m and auv3_extension_editor.m.
// The editor sync of the bridge reads the messages of the page and builds
// every script. These static functions run its scripts in the WebView the
// same way in all AU format variants. Each template only does in the host
// what the web UI edits, through its BeamerAuEditorHost.

#pragma once

#include "BeamerAuBridge.h"

/// Evaluate a script that Rust built in the WebView, then free it.
static void beamer_au_ipc_eval_script(void* webviewHandle, char* script) {
    if (script && webviewHandle) {
        beamer_webview_eval_js(webviewHandle, (const uint8_t*)script, strlen(script));
    }
    beamer_au_free_string(script);
}

/// Hand a message of the page to the editor sync and run its answer.
///
/// `host` does in the host what the message edits. Without an editor sync
/// the message goes nowhere.
static void beamer_au_ipc_handle_message(
    BeamerAuEditorSyncHandle sync,
    void* webviewHandle,
    const uint8_t* json,
    size_t len,
    const BeamerAuEditorHost* host
) {
    if (!sync) return;
    beamer_au_ipc_eval_script(webviewHandle, beamer_au_editor_sync_message(sync, json, len, host));
}

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
