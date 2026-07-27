#ifndef TSUMUJI_PHONE_PJSIP_H
#define TSUMUJI_PHONE_PJSIP_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct tp_engine tp_engine;

typedef struct tp_config {
    const char *server;
    const char *local_address;
    const char *user_id;
    const char *password;
    uint16_t sip_port;
    uint16_t rtp_port_start;
    uint16_t rtp_port_end;
} tp_config;

typedef enum tp_event_kind {
    TP_EVENT_REGISTERING = 1,
    TP_EVENT_REGISTERED = 2,
    TP_EVENT_REGISTRATION_FAILED = 3,
    TP_EVENT_INCOMING = 4,
    TP_EVENT_DIALING = 5,
    TP_EVENT_CONNECTING = 6,
    TP_EVENT_ACTIVE = 7,
    TP_EVENT_ENDED = 8,
    TP_EVENT_ERROR = 9
} tp_event_kind;

typedef struct tp_event {
    tp_event_kind kind;
    int32_t call_id;
    const char *peer;
    const char *message;
} tp_event;

typedef void (*tp_event_callback)(void *user_data, const tp_event *event);

void tp_android_set_jvm(void *java_vm);
int tp_engine_start(const tp_config *config,
                    tp_event_callback callback,
                    void *user_data,
                    tp_engine **out_engine);
int tp_engine_stop(tp_engine *engine);
int tp_engine_reregister(tp_engine *engine);
int tp_engine_dial(tp_engine *engine, const char *number);
int tp_engine_answer(tp_engine *engine);
int tp_engine_reject(tp_engine *engine);
int tp_engine_hang_up(tp_engine *engine);
int tp_engine_send_dtmf(tp_engine *engine, char digit);
int tp_engine_set_muted(tp_engine *engine, int muted);
const char *tp_last_error(const tp_engine *engine);

#ifdef __cplusplus
}
#endif

#endif
