// Channel layout helpers for the AU wrappers.
//
// Included by auv2_wrapper.c and auv3_wrapper.m. A host describes the channels of a bus with
// an AudioChannelLayout: a layout tag, a channel bitmap or one description
// per channel. These functions expand it into the channel label of each
// channel, which the bridge matches against the plugin's channel sets. They
// also write a layout back for the host.

#pragma once

#include <AudioToolbox/AudioToolbox.h>
#include <string.h>

#include "BeamerAuBridge.h"

// A bus's channel layout as the host set it: its tag and the label of each
// channel. A count of 0 means the host set none.
typedef struct {
    AudioChannelLayoutTag tag;
    UInt32 count;
    UInt32 labels[BEAMER_AU_MAX_CHANNELS];
} BeamerAuLayout;

// Bytes of an AudioChannelLayout with `descriptions` channel descriptions.
static UInt32 beamer_au_layout_bytes(UInt32 descriptions) {
    return (UInt32)offsetof(AudioChannelLayout, mChannelDescriptions)
        + descriptions * (UInt32)sizeof(AudioChannelDescription);
}

// Expand `layout` (`size` bytes) into `out`. Returns false for a layout
// without channels, with more than BEAMER_AU_MAX_CHANNELS or one that
// cannot be read.
static bool beamer_au_layout_read(const AudioChannelLayout* layout, UInt32 size, BeamerAuLayout* out) {
    if (layout == NULL || size < beamer_au_layout_bytes(0)) {
        return false;
    }
    memset(out, 0, sizeof(*out));
    out->tag = layout->mChannelLayoutTag;

    // A layout of descriptions has them in it; a tag or a bitmap is
    // expanded by the system.
    const AudioChannelLayout* expanded = layout;
    AudioChannelLayout* own = NULL;
    if (layout->mChannelLayoutTag == kAudioChannelLayoutTag_UseChannelDescriptions) {
        if (layout->mNumberChannelDescriptions > BEAMER_AU_MAX_CHANNELS
            || size < beamer_au_layout_bytes(layout->mNumberChannelDescriptions)) {
            return false;
        }
    } else {
        AudioFormatPropertyID property = kAudioFormatProperty_ChannelLayoutForTag;
        const void* specifier = &layout->mChannelLayoutTag;
        UInt32 specifierSize = sizeof(layout->mChannelLayoutTag);
        if (layout->mChannelLayoutTag == kAudioChannelLayoutTag_UseChannelBitmap) {
            property = kAudioFormatProperty_ChannelLayoutForBitmap;
            specifier = &layout->mChannelBitmap;
            specifierSize = sizeof(layout->mChannelBitmap);
        }
        UInt32 expandedSize = 0;
        if (AudioFormatGetPropertyInfo(property, specifierSize, specifier, &expandedSize) != noErr
            || expandedSize < beamer_au_layout_bytes(0)) {
            return false;
        }
        own = (AudioChannelLayout*)calloc(1, expandedSize);
        if (own == NULL
            || AudioFormatGetProperty(property, specifierSize, specifier, &expandedSize, own) != noErr
            || own->mNumberChannelDescriptions > BEAMER_AU_MAX_CHANNELS
            || expandedSize < beamer_au_layout_bytes(own->mNumberChannelDescriptions)) {
            free(own);
            return false;
        }
        expanded = own;
    }

    UInt32 count = expanded->mNumberChannelDescriptions;
    bool fits = count > 0 && count <= BEAMER_AU_MAX_CHANNELS;
    if (fits) {
        out->count = count;
        for (UInt32 i = 0; i < count; i++) {
            out->labels[i] = expanded->mChannelDescriptions[i].mChannelLabel;
        }
    }
    free(own);
    return fits;
}

// Bytes `layout` takes as an AudioChannelLayout: a tag alone, or one
// description per channel for a layout of descriptions.
static UInt32 beamer_au_layout_size(const BeamerAuLayout* layout) {
    bool described = layout->tag == kAudioChannelLayoutTag_UseChannelDescriptions;
    return beamer_au_layout_bytes(described ? layout->count : 0);
}

// Write `layout` into `out`, which holds beamer_au_layout_size(layout)
// bytes. A layout from a bitmap is written as its descriptions.
static void beamer_au_layout_write(const BeamerAuLayout* layout, AudioChannelLayout* out) {
    memset(out, 0, beamer_au_layout_size(layout));
    out->mChannelLayoutTag = layout->tag;
    if (layout->tag == kAudioChannelLayoutTag_UseChannelDescriptions) {
        out->mNumberChannelDescriptions = layout->count;
        for (UInt32 i = 0; i < layout->count; i++) {
            out->mChannelDescriptions[i].mChannelLabel = layout->labels[i];
        }
    }
}

// The layout to keep for a host layout: a bitmap becomes descriptions, so
// that it can be written back in the same form.
static void beamer_au_layout_keep(BeamerAuLayout* layout) {
    if (layout->tag == kAudioChannelLayoutTag_UseChannelBitmap) {
        layout->tag = kAudioChannelLayoutTag_UseChannelDescriptions;
    }
}

// A layout of only a tag, for a bus the host set no layout on.
static BeamerAuLayout beamer_au_layout_of_tag(AudioChannelLayoutTag tag) {
    BeamerAuLayout layout;
    memset(&layout, 0, sizeof(layout));
    layout.tag = tag;
    return layout;
}

// Copy the labels of `layout` into `bus` when they describe its channels:
// the host set a layout with the bus's channel count.
static void beamer_au_layout_to_bus(const BeamerAuLayout* layout, BeamerAuBusInfo* bus) {
    if (layout->count > 0 && layout->count == bus->channel_count) {
        bus->layout_label_count = layout->count;
        memcpy(bus->layout_labels, layout->labels, layout->count * sizeof(UInt32));
    }
}
