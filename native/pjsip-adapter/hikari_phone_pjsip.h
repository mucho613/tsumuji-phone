#ifndef HIKARI_PHONE_PJSIP_H
#define HIKARI_PHONE_PJSIP_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct hp_engine hp_engine;

typedef struct hp_config {
    const char *server;
    const char *local_address;
    const char *user_id;
    const char *password;
    uint16_t sip_port;
    uint16_t rtp_port_start;
    uint16_t rtp_port_end;
} hp_config;

typedef enum hp_event_kind {
    HP_EVENT_REGISTERING = 1,
    HP_EVENT_REGISTERED = 2,
    HP_EVENT_REGISTRATION_FAILED = 3,
    HP_EVENT_INCOMING = 4,
    HP_EVENT_DIALING = 5,
    HP_EVENT_CONNECTING = 6,
    HP_EVENT_ACTIVE = 7,
    HP_EVENT_ENDED = 8,
    HP_EVENT_ERROR = 9
} hp_event_kind;

typedef struct hp_event {
    hp_event_kind kind;
    int32_t call_id;
    const char *peer;
    const char *message;
} hp_event;

typedef void (*hp_event_callback)(void *user_data, const hp_event *event);

int hp_engine_start(const hp_config *config,
                    hp_event_callback callback,
                    void *user_data,
                    hp_engine **out_engine);
int hp_engine_stop(hp_engine *engine);
int hp_engine_dial(hp_engine *engine, const char *number);
int hp_engine_answer(hp_engine *engine);
int hp_engine_reject(hp_engine *engine);
int hp_engine_hang_up(hp_engine *engine);
int hp_engine_send_dtmf(hp_engine *engine, char digit);
int hp_engine_set_muted(hp_engine *engine, int muted);
const char *hp_last_error(const hp_engine *engine);

#ifdef __cplusplus
}
#endif

#endif
