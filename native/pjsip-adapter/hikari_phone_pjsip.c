#include "hikari_phone_pjsip.h"

#include <pjsua-lib/pjsua.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

struct hp_engine {
    pjsua_acc_id account_id;
    pjsua_call_id call_id;
    hp_event_callback callback;
    void *user_data;
    char server[256];
    char last_error[256];
    int started;
    int muted;
};

static hp_engine *active_engine;

static pj_str_t hp_pj_str(const char *value) {
    return pj_str((char *)value);
}

static void hp_copy_pj_string(char *destination,
                              size_t destination_size,
                              const pj_str_t *source) {
    size_t length;
    if (destination_size == 0) {
        return;
    }
    length = source->slen > 0 ? (size_t)source->slen : 0;
    if (length >= destination_size) {
        length = destination_size - 1;
    }
    if (length != 0) {
        memcpy(destination, source->ptr, length);
    }
    destination[length] = '\0';
}

static void hp_emit(hp_event_kind kind,
                    pjsua_call_id call_id,
                    const char *peer,
                    const char *message) {
    hp_event event;
    if (active_engine == NULL || active_engine->callback == NULL) {
        return;
    }
    event.kind = kind;
    event.call_id = call_id;
    event.peer = peer != NULL ? peer : "";
    event.message = message != NULL ? message : "";
    active_engine->callback(active_engine->user_data, &event);
}

static int hp_fail(hp_engine *engine, pj_status_t status, const char *operation) {
    char detail[PJ_ERR_MSG_SIZE];
    pj_strerror(status, detail, sizeof(detail));
    snprintf(engine->last_error,
             sizeof(engine->last_error),
             "%s: %s (%d)",
             operation,
             detail,
             status);
    hp_emit(HP_EVENT_ERROR, engine->call_id, "", engine->last_error);
    return status != PJ_SUCCESS ? status : -1;
}

static void hp_on_incoming_call(pjsua_acc_id account_id,
                                pjsua_call_id call_id,
                                pjsip_rx_data *received_data) {
    pjsua_call_info info;
    char peer[PJSIP_MAX_URL_SIZE];
    PJ_UNUSED_ARG(account_id);
    PJ_UNUSED_ARG(received_data);

    if (active_engine == NULL) {
        pjsua_call_answer(call_id, PJSIP_SC_BUSY_HERE, NULL, NULL);
        return;
    }
    if (active_engine->call_id != PJSUA_INVALID_ID) {
        pjsua_call_answer(call_id, PJSIP_SC_BUSY_HERE, NULL, NULL);
        return;
    }
    active_engine->call_id = call_id;
    peer[0] = '\0';
    if (pjsua_call_get_info(call_id, &info) == PJ_SUCCESS) {
        hp_copy_pj_string(peer, sizeof(peer), &info.remote_info);
    }
    hp_emit(HP_EVENT_INCOMING, call_id, peer, "");
}

static void hp_on_call_state(pjsua_call_id call_id, pjsip_event *event) {
    pjsua_call_info info;
    char peer[PJSIP_MAX_URL_SIZE];
    char status_text[96];
    char reason[128];
    PJ_UNUSED_ARG(event);

    if (active_engine == NULL ||
        pjsua_call_get_info(call_id, &info) != PJ_SUCCESS) {
        return;
    }
    hp_copy_pj_string(peer, sizeof(peer), &info.remote_info);
    hp_copy_pj_string(status_text, sizeof(status_text), &info.last_status_text);
    snprintf(reason, sizeof(reason), "%d %s", info.last_status, status_text);

    switch (info.state) {
    case PJSIP_INV_STATE_CALLING:
        hp_emit(HP_EVENT_DIALING, call_id, peer, "");
        break;
    case PJSIP_INV_STATE_INCOMING:
        break;
    case PJSIP_INV_STATE_EARLY:
    case PJSIP_INV_STATE_CONNECTING:
        hp_emit(HP_EVENT_CONNECTING, call_id, peer, "");
        break;
    case PJSIP_INV_STATE_CONFIRMED:
        hp_emit(HP_EVENT_ACTIVE, call_id, peer, "");
        break;
    case PJSIP_INV_STATE_DISCONNECTED:
        hp_emit(HP_EVENT_ENDED, call_id, peer, reason);
        if (active_engine->call_id == call_id) {
            active_engine->call_id = PJSUA_INVALID_ID;
            active_engine->muted = 0;
        }
        break;
    default:
        break;
    }
}

static void hp_on_call_media_state(pjsua_call_id call_id) {
    pjsua_call_info info;
    if (pjsua_call_get_info(call_id, &info) != PJ_SUCCESS ||
        info.media_status != PJSUA_CALL_MEDIA_ACTIVE) {
        return;
    }
    pjsua_conf_connect(info.conf_slot, 0);
    if (active_engine == NULL || !active_engine->muted) {
        pjsua_conf_connect(0, info.conf_slot);
    }
}

static void hp_on_reg_state(pjsua_acc_id account_id) {
    pjsua_acc_info info;
    char reason[128];
    if (pjsua_acc_get_info(account_id, &info) != PJ_SUCCESS) {
        hp_emit(HP_EVENT_REGISTRATION_FAILED, PJSUA_INVALID_ID, "", "status unavailable");
        return;
    }
    if (info.status == PJSIP_SC_OK) {
        hp_emit(HP_EVENT_REGISTERED, PJSUA_INVALID_ID, "", "");
    } else if (info.status >= 300) {
        hp_copy_pj_string(reason, sizeof(reason), &info.status_text);
        hp_emit(HP_EVENT_REGISTRATION_FAILED, PJSUA_INVALID_ID, "", reason);
    } else {
        hp_emit(HP_EVENT_REGISTERING, PJSUA_INVALID_ID, "", "");
    }
}

static pj_status_t hp_enable_pcmu_only(void) {
    pjsua_codec_info codecs[64];
    unsigned count = PJ_ARRAY_SIZE(codecs);
    unsigned index;
    pj_status_t status = pjsua_enum_codecs(codecs, &count);
    if (status != PJ_SUCCESS) {
        return status;
    }
    for (index = 0; index < count; ++index) {
        pj_uint8_t priority = 0;
        if (pj_stricmp2(&codecs[index].codec_id, "PCMU/8000/1") == 0 ||
            pj_stricmp2(&codecs[index].codec_id, "PCMU/8000") == 0) {
            priority = PJMEDIA_CODEC_PRIO_HIGHEST;
        }
        status = pjsua_codec_set_priority(&codecs[index].codec_id, priority);
        if (status != PJ_SUCCESS) {
            return status;
        }
    }
    return PJ_SUCCESS;
}

int hp_engine_start(const hp_config *config,
                    hp_event_callback callback,
                    void *user_data,
                    hp_engine **out_engine) {
    hp_engine *engine;
    pjsua_config user_agent_config;
    pjsua_logging_config logging_config;
    pjsua_media_config media_config;
    pjsua_transport_config transport_config;
    pjsua_acc_config account_config;
    pjsua_transport_id transport_id;
    pj_status_t status;
    char identity[512];
    char registrar[512];

    if (config == NULL || callback == NULL || out_engine == NULL ||
        config->server == NULL || config->local_address == NULL ||
        config->user_id == NULL || config->password == NULL ||
        active_engine != NULL) {
        return -1;
    }

    engine = calloc(1, sizeof(*engine));
    if (engine == NULL) {
        return -1;
    }
    engine->account_id = PJSUA_INVALID_ID;
    engine->call_id = PJSUA_INVALID_ID;
    engine->callback = callback;
    engine->user_data = user_data;
    snprintf(engine->server, sizeof(engine->server), "%s", config->server);
    active_engine = engine;

    status = pjsua_create();
    if (status != PJ_SUCCESS) {
        hp_fail(engine, status, "pjsua_create");
        goto fail;
    }

    pjsua_config_default(&user_agent_config);
    user_agent_config.max_calls = 1;
    user_agent_config.cb.on_incoming_call = &hp_on_incoming_call;
    user_agent_config.cb.on_call_state = &hp_on_call_state;
    user_agent_config.cb.on_call_media_state = &hp_on_call_media_state;
    user_agent_config.cb.on_reg_state = &hp_on_reg_state;
    user_agent_config.user_agent = hp_pj_str("Hikari Phone/0.1");

    pjsua_logging_config_default(&logging_config);
    logging_config.msg_logging = PJ_FALSE;
    logging_config.level = 2;
    logging_config.console_level = 2;

    pjsua_media_config_default(&media_config);
    media_config.clock_rate = 8000;
    media_config.snd_clock_rate = 8000;
    media_config.channel_count = 1;
    media_config.audio_frame_ptime = 20;
    media_config.ec_tail_len = 200;
    media_config.no_vad = PJ_TRUE;

    status = pjsua_init(&user_agent_config, &logging_config, &media_config);
    if (status != PJ_SUCCESS) {
        hp_fail(engine, status, "pjsua_init");
        goto fail;
    }

    pjsua_transport_config_default(&transport_config);
    transport_config.port = config->sip_port;
    transport_config.bound_addr = hp_pj_str(config->local_address);
    status = pjsua_transport_create(PJSIP_TRANSPORT_UDP,
                                    &transport_config,
                                    &transport_id);
    if (status != PJ_SUCCESS) {
        hp_fail(engine, status, "pjsua_transport_create");
        goto fail;
    }

    status = pjsua_start();
    if (status != PJ_SUCCESS) {
        hp_fail(engine, status, "pjsua_start");
        goto fail;
    }
    status = hp_enable_pcmu_only();
    if (status != PJ_SUCCESS) {
        hp_fail(engine, status, "PCMU codec configuration");
        goto fail;
    }

    pjsua_acc_config_default(&account_config);
    snprintf(identity,
             sizeof(identity),
             "sip:%s@%s",
             config->user_id,
             config->server);
    snprintf(registrar, sizeof(registrar), "sip:%s", config->server);
    account_config.id = hp_pj_str(identity);
    account_config.reg_uri = hp_pj_str(registrar);
    account_config.transport_id = transport_id;
    account_config.cred_count = 1;
    account_config.cred_info[0].realm = hp_pj_str("*");
    account_config.cred_info[0].scheme = hp_pj_str("Digest");
    account_config.cred_info[0].username = hp_pj_str(config->user_id);
    account_config.cred_info[0].data_type = PJSIP_CRED_DATA_PLAIN_PASSWD;
    account_config.cred_info[0].data = hp_pj_str(config->password);
    account_config.rtp_cfg.port = config->rtp_port_start;
    account_config.rtp_cfg.port_range =
        config->rtp_port_end - config->rtp_port_start;
    account_config.rtp_cfg.bound_addr = hp_pj_str(config->local_address);

    hp_emit(HP_EVENT_REGISTERING, PJSUA_INVALID_ID, "", "");
    status = pjsua_acc_add(&account_config, PJ_TRUE, &engine->account_id);
    if (status != PJ_SUCCESS) {
        hp_fail(engine, status, "pjsua_acc_add");
        goto fail;
    }

    engine->started = 1;
    *out_engine = engine;
    return 0;

fail:
    pjsua_destroy();
    active_engine = NULL;
    free(engine);
    return status != PJ_SUCCESS ? status : -1;
}

int hp_engine_stop(hp_engine *engine) {
    int status;
    if (engine == NULL) {
        return 0;
    }
    if (engine != active_engine) {
        return -1;
    }
    status = pjsua_destroy();
    active_engine = NULL;
    free(engine);
    return status;
}

int hp_engine_dial(hp_engine *engine, const char *number) {
    char destination[512];
    pj_str_t uri;
    pjsua_call_setting call_setting;
    pj_status_t status;
    if (engine == NULL || number == NULL ||
        engine->call_id != PJSUA_INVALID_ID) {
        return -1;
    }
    snprintf(destination, sizeof(destination), "sip:%s@%s", number, engine->server);
    uri = hp_pj_str(destination);
    pjsua_call_setting_default(&call_setting);
    call_setting.aud_cnt = 1;
    call_setting.vid_cnt = 0;
    call_setting.txt_cnt = 0;
    status = pjsua_call_make_call(engine->account_id,
                                  &uri,
                                  &call_setting,
                                  NULL,
                                  NULL,
                                  &engine->call_id);
    return status == PJ_SUCCESS ? 0 : hp_fail(engine, status, "pjsua_call_make_call");
}

int hp_engine_answer(hp_engine *engine) {
    pj_status_t status;
    if (engine == NULL || engine->call_id == PJSUA_INVALID_ID) {
        return -1;
    }
    status = pjsua_call_answer(engine->call_id, PJSIP_SC_OK, NULL, NULL);
    return status == PJ_SUCCESS ? 0 : hp_fail(engine, status, "pjsua_call_answer");
}

int hp_engine_reject(hp_engine *engine) {
    pj_status_t status;
    if (engine == NULL || engine->call_id == PJSUA_INVALID_ID) {
        return -1;
    }
    status = pjsua_call_answer(engine->call_id, PJSIP_SC_BUSY_HERE, NULL, NULL);
    return status == PJ_SUCCESS ? 0 : hp_fail(engine, status, "pjsua_call_answer(486)");
}

int hp_engine_hang_up(hp_engine *engine) {
    pj_status_t status;
    if (engine == NULL || engine->call_id == PJSUA_INVALID_ID) {
        return -1;
    }
    status = pjsua_call_hangup(engine->call_id, 0, NULL, NULL);
    return status == PJ_SUCCESS ? 0 : hp_fail(engine, status, "pjsua_call_hangup");
}

int hp_engine_send_dtmf(hp_engine *engine, char digit) {
    char digits[2] = {digit, '\0'};
    pj_str_t value = hp_pj_str(digits);
    pj_status_t status;
    if (engine == NULL || engine->call_id == PJSUA_INVALID_ID ||
        strchr("0123456789*#", digit) == NULL) {
        return -1;
    }
    status = pjsua_call_dial_dtmf(engine->call_id, &value);
    return status == PJ_SUCCESS ? 0 : hp_fail(engine, status, "pjsua_call_dial_dtmf");
}

int hp_engine_set_muted(hp_engine *engine, int muted) {
    pjsua_conf_port_id call_port;
    pj_status_t status;
    if (engine == NULL || engine->call_id == PJSUA_INVALID_ID) {
        return -1;
    }
    call_port = pjsua_call_get_conf_port(engine->call_id);
    if (call_port == PJSUA_INVALID_ID) {
        return -1;
    }
    status = muted ? pjsua_conf_disconnect(0, call_port)
                   : pjsua_conf_connect(0, call_port);
    if (status == PJ_SUCCESS) {
        engine->muted = muted != 0;
        return 0;
    }
    return hp_fail(engine, status, "microphone mute");
}

const char *hp_last_error(const hp_engine *engine) {
    return engine != NULL ? engine->last_error : "PJSIP engine is not running";
}
