#pragma once

#include <AudioServerPlugIn.h>
#include <CoreAudio/CoreAudio.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct LoopbackMixer* LoopbackMixerHandle;

typedef struct LoopbackLevels {
    float inputs[8];
    float outputs[8];
    uint32_t input_count;
    uint32_t output_count;
} LoopbackLevels;

typedef struct LoopbackRenderArgs {
    AudioBufferList* bufferList;
    uint32_t frameCount;
    const AudioTimeStamp* timestamp;
} LoopbackRenderArgs;

LoopbackMixerHandle loopback_mixer_create(double sampleRate, uint32_t maxFrames);
void loopback_mixer_destroy(LoopbackMixerHandle handle);
OSStatus loopback_mixer_process(LoopbackMixerHandle handle, const LoopbackRenderArgs* args);
void loopback_mixer_set_gain(LoopbackMixerHandle handle, uint32_t sourceIndex, float gain);
void loopback_mixer_set_mute(LoopbackMixerHandle handle, uint32_t sourceIndex, bool mute);
void loopback_mixer_submit_input(LoopbackMixerHandle handle, const float* data, uint32_t frames);
bool loopback_mixer_register_node_source(LoopbackMixerHandle handle, uint32_t sourceIndex, uint32_t capacityFrames);
bool loopback_mixer_push_node_frames(LoopbackMixerHandle handle, uint32_t sourceIndex, const float* data, uint32_t frames, uint64_t timestamp_ns);
bool loopback_mixer_set_node_gain(LoopbackMixerHandle handle, uint32_t sourceIndex, float gain);
bool loopback_mixer_set_node_mute(LoopbackMixerHandle handle, uint32_t sourceIndex, bool mute);
LoopbackMixerHandle loopback_mixer_global_handle(void);

bool device_kit_get_levels(LoopbackLevels* levels_out);
double device_kit_current_sample_rate(void);
uint32_t device_kit_buffer_size_frames(void);
double device_kit_latency_ms(void);
bool device_kit_start_driver(void);
void device_kit_stop_driver(void);
bool device_kit_start_engine(void);
void device_kit_stop_engine(void);
uint32_t device_kit_source_count(void);
bool device_kit_source_is_enabled(uint32_t sourceIndex);
void device_kit_set_source_enabled(uint32_t sourceIndex, bool enabled);
const char* device_kit_pop_log(void);

#ifdef __cplusplus
}
#endif
