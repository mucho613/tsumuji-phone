#include "tsumuji_phone_pjsip.h"

#include <pjsip/sip_config.h>
#include <pjsua-lib/pjsua.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

struct tp_engine {
    pjsua_acc_id account_id;
    pjsua_call_id call_id;
    tp_event_callback callback;
    void *user_data;
    char server[256];
    char last_error[256];
    int started;
    int muted;
};

static tp_engine *active_engine;

#if defined(_MSC_VER)
#define TP_THREAD_LOCAL __declspec(thread)
#else
#define TP_THREAD_LOCAL __thread
#endif

static TP_THREAD_LOCAL pj_thread_desc external_thread_desc;
static TP_THREAD_LOCAL pj_thread_t *external_thread;

#if defined(PJ_ANDROID) && PJ_ANDROID
void tp_android_set_jvm(void *java_vm) {
    pj_jni_set_jvm(java_vm);
}
#endif

static pj_str_t tp_pj_str(const char *value) {
    return pj_str((char *)value);
}

static void tp_copy_pj_string(char *destination,
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

static void tp_emit(tp_event_kind kind,
                    pjsua_call_id call_id,
                    const char *peer,
                    const char *message) {
    tp_event event;
    if (active_engine == NULL || active_engine->callback == NULL) {
        return;
    }
    event.kind = kind;
    event.call_id = call_id;
    event.peer = peer != NULL ? peer : "";
    event.message = message != NULL ? message : "";
    active_engine->callback(active_engine->user_data, &event);
}

static int tp_fail(tp_engine *engine, pj_status_t status, const char *operation) {
    char detail[PJ_ERR_MSG_SIZE];
    pj_strerror(status, detail, sizeof(detail));
    snprintf(engine->last_error,
             sizeof(engine->last_error),
             "%s: %s (%d)",
             operation,
             detail,
             status);
    tp_emit(TP_EVENT_ERROR, engine->call_id, "", engine->last_error);
    return status != PJ_SUCCESS ? status : -1;
}

static int tp_ensure_thread_registered(tp_engine *engine) {
    pj_status_t status;
    if (pj_thread_is_registered()) {
        return 0;
    }
    memset(external_thread_desc, 0, sizeof(external_thread_desc));
    status = pj_thread_register("tsumuji-api",
                                external_thread_desc,
                                &external_thread);
    return status == PJ_SUCCESS
               ? 0
               : tp_fail(engine, status, "pj_thread_register");
}

static void tp_on_incoming_call(pjsua_acc_id account_id,
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
        tp_copy_pj_string(peer, sizeof(peer), &info.remote_info);
    }
    tp_emit(TP_EVENT_INCOMING, call_id, peer, "");
}

static void tp_on_call_state(pjsua_call_id call_id, pjsip_event *event) {
    pjsua_call_info info;
    char peer[PJSIP_MAX_URL_SIZE];
    char status_text[96];
    char reason[128];
    PJ_UNUSED_ARG(event);

    if (active_engine == NULL ||
        pjsua_call_get_info(call_id, &info) != PJ_SUCCESS) {
        return;
    }
    tp_copy_pj_string(peer, sizeof(peer), &info.remote_info);
    tp_copy_pj_string(status_text, sizeof(status_text), &info.last_status_text);
    snprintf(reason, sizeof(reason), "%d %s", info.last_status, status_text);

    switch (info.state) {
    case PJSIP_INV_STATE_CALLING:
        tp_emit(TP_EVENT_DIALING, call_id, peer, "");
        break;
    case PJSIP_INV_STATE_INCOMING:
        break;
    case PJSIP_INV_STATE_EARLY:
    case PJSIP_INV_STATE_CONNECTING:
        tp_emit(TP_EVENT_CONNECTING, call_id, peer, "");
        break;
    case PJSIP_INV_STATE_CONFIRMED:
        tp_emit(TP_EVENT_ACTIVE, call_id, peer, "");
        break;
    case PJSIP_INV_STATE_DISCONNECTED:
        tp_emit(TP_EVENT_ENDED, call_id, peer, reason);
        if (active_engine->call_id == call_id) {
            active_engine->call_id = PJSUA_INVALID_ID;
            active_engine->muted = 0;
        }
        break;
    default:
        break;
    }
}

static void tp_on_call_media_state(pjsua_call_id call_id) {
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

static void tp_on_reg_state(pjsua_acc_id account_id) {
    pjsua_acc_info info;
    char reason[128];
    if (pjsua_acc_get_info(account_id, &info) != PJ_SUCCESS) {
        tp_emit(TP_EVENT_REGISTRATION_FAILED, PJSUA_INVALID_ID, "", "status unavailable");
        return;
    }
    if (info.status == PJSIP_SC_OK) {
        tp_emit(TP_EVENT_REGISTERED, PJSUA_INVALID_ID, "", "");
    } else if (info.status >= 300) {
        tp_copy_pj_string(reason, sizeof(reason), &info.status_text);
        tp_emit(TP_EVENT_REGISTRATION_FAILED, PJSUA_INVALID_ID, "", reason);
    } else {
        tp_emit(TP_EVENT_REGISTERING, PJSUA_INVALID_ID, "", "");
    }
}

static pj_status_t tp_enable_pcmu_only(void) {
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

int tp_engine_start(const tp_config *config,
                    tp_event_callback callback,
                    void *user_data,
                    tp_engine **out_engine) {
    tp_engine *engine;
    pjsua_config user_agent_config;
    pjsua_logging_config logging_config;
    pjsua_media_config media_config;
    pjsua_transport_config transport_config;
    pjsua_acc_config account_config;
    pjsua_transport_id transport_id;
    pj_status_t status;
    char identity[512];
    char registrar[512];
    char push_contact_params[256];

    if (config == NULL || callback == NULL || out_engine == NULL ||
        config->server == NULL || config->local_address == NULL ||
        config->user_id == NULL || config->password == NULL ||
        config->push_device_id == NULL ||
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
        tp_fail(engine, status, "pjsua_create");
        goto fail;
    }

    /* Some PSTN services provide their announcement entirely as early media
     * and never answer the SIP dialog with a final 2xx response. Keep that
     * usable INVITE session alive after the transaction timer expires; the
     * user can still end it explicitly through pjsua_call_hangup(). */
    pjsip_cfg()->endpt.keep_inv_after_tsx_timeout = PJ_TRUE;

    pjsua_config_default(&user_agent_config);
    user_agent_config.max_calls = 1;
    user_agent_config.cb.on_incoming_call = &tp_on_incoming_call;
    user_agent_config.cb.on_call_state = &tp_on_call_state;
    user_agent_config.cb.on_call_media_state = &tp_on_call_media_state;
    user_agent_config.cb.on_reg_state = &tp_on_reg_state;
    user_agent_config.user_agent = tp_pj_str("Tsumuji Phone/0.1");

    pjsua_logging_config_default(&logging_config);
    logging_config.msg_logging = PJ_FALSE;
    logging_config.level = 2;
    logging_config.console_level = 2;

    pjsua_media_config_default(&media_config);
    media_config.clock_rate = 8000;
#if defined(PJ_ANDROID) && PJ_ANDROID
    media_config.snd_clock_rate = 48000;
#else
    media_config.snd_clock_rate = 8000;
#endif
    media_config.channel_count = 1;
    media_config.audio_frame_ptime = 20;
    media_config.ec_tail_len = 200;
    media_config.no_vad = PJ_TRUE;

    status = pjsua_init(&user_agent_config, &logging_config, &media_config);
    if (status != PJ_SUCCESS) {
        tp_fail(engine, status, "pjsua_init");
        goto fail;
    }

    pjsua_transport_config_default(&transport_config);
    transport_config.port = config->sip_port;
    transport_config.bound_addr = tp_pj_str(config->local_address);
    status = pjsua_transport_create(PJSIP_TRANSPORT_UDP,
                                    &transport_config,
                                    &transport_id);
    if (status != PJ_SUCCESS) {
        tp_fail(engine, status, "pjsua_transport_create");
        goto fail;
    }

    status = pjsua_start();
    if (status != PJ_SUCCESS) {
        tp_fail(engine, status, "pjsua_start");
        goto fail;
    }
    status = tp_enable_pcmu_only();
    if (status != PJ_SUCCESS) {
        tp_fail(engine, status, "PCMU codec configuration");
        goto fail;
    }

    pjsua_acc_config_default(&account_config);
    snprintf(identity,
             sizeof(identity),
             "sip:%s@%s",
             config->user_id,
             config->server);
    snprintf(registrar, sizeof(registrar), "sip:%s", config->server);
    account_config.id = tp_pj_str(identity);
    account_config.reg_uri = tp_pj_str(registrar);
    account_config.transport_id = transport_id;
    account_config.cred_count = 1;
    account_config.cred_info[0].realm = tp_pj_str("*");
    account_config.cred_info[0].scheme = tp_pj_str("Digest");
    account_config.cred_info[0].username = tp_pj_str(config->user_id);
    account_config.cred_info[0].data_type = PJSIP_CRED_DATA_PLAIN_PASSWD;
    account_config.cred_info[0].data = tp_pj_str(config->password);
    if (config->push_device_id[0] != '\0') {
        snprintf(push_contact_params,
                 sizeof(push_contact_params),
                 ";pn-provider=fcm;pn-prid=%s;pn-param=app.tsumujiphone.local",
                 config->push_device_id);
        account_config.reg_contact_uri_params = tp_pj_str(push_contact_params);
        account_config.reg_timeout = 30U * 24U * 60U * 60U;
    }
#if defined(__ANDROID__)
    account_config.reg_retry_interval = 0;
    account_config.reg_first_retry_interval = 0;
    account_config.reg_retry_random_interval = 0;
#endif
    account_config.rtp_cfg.port = config->rtp_port_start;
    account_config.rtp_cfg.port_range =
        config->rtp_port_end - config->rtp_port_start;
    account_config.rtp_cfg.bound_addr = tp_pj_str(config->local_address);

    tp_emit(TP_EVENT_REGISTERING, PJSUA_INVALID_ID, "", "");
    status = pjsua_acc_add(&account_config, PJ_TRUE, &engine->account_id);
    if (status != PJ_SUCCESS) {
        tp_fail(engine, status, "pjsua_acc_add");
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

static int tp_engine_stop_with_flags(tp_engine *engine, unsigned flags) {
    int status;
    if (engine == NULL) {
        return 0;
    }
    if (engine != active_engine) {
        return -1;
    }
    status = tp_ensure_thread_registered(engine);
    if (status != 0) {
        return status;
    }
    status = pjsua_destroy2(flags);
    active_engine = NULL;
    free(engine);
    return status;
}

int tp_engine_stop(tp_engine *engine) {
    return tp_engine_stop_with_flags(engine, 0);
}

int tp_engine_stop_dormant(tp_engine *engine) {
    return tp_engine_stop_with_flags(engine, PJSUA_DESTROY_NO_TX_MSG);
}

int tp_engine_reregister(tp_engine *engine) {
    pj_status_t status;
    if (engine == NULL || engine != active_engine ||
        engine->account_id == PJSUA_INVALID_ID) {
        return -1;
    }
    status = tp_ensure_thread_registered(engine);
    if (status != 0) {
        return status;
    }
    status = pjsua_acc_set_registration(engine->account_id, PJ_TRUE);
    return status == PJ_SUCCESS
               ? 0
               : tp_fail(engine, status, "pjsua_acc_set_registration");
}

int tp_engine_dial(tp_engine *engine, const char *number) {
    char destination[512];
    pj_str_t uri;
    pjsua_call_setting call_setting;
    pj_status_t status;
    if (engine == NULL || number == NULL ||
        engine->call_id != PJSUA_INVALID_ID) {
        return -1;
    }
    status = tp_ensure_thread_registered(engine);
    if (status != 0) {
        return status;
    }
    snprintf(destination, sizeof(destination), "sip:%s@%s", number, engine->server);
    uri = tp_pj_str(destination);
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
    return status == PJ_SUCCESS ? 0 : tp_fail(engine, status, "pjsua_call_make_call");
}

int tp_engine_answer(tp_engine *engine) {
    pj_status_t status;
    if (engine == NULL || engine->call_id == PJSUA_INVALID_ID) {
        return -1;
    }
    status = tp_ensure_thread_registered(engine);
    if (status != 0) {
        return status;
    }
    status = pjsua_call_answer(engine->call_id, PJSIP_SC_OK, NULL, NULL);
    return status == PJ_SUCCESS ? 0 : tp_fail(engine, status, "pjsua_call_answer");
}

int tp_engine_reject(tp_engine *engine) {
    pj_status_t status;
    if (engine == NULL || engine->call_id == PJSUA_INVALID_ID) {
        return -1;
    }
    status = tp_ensure_thread_registered(engine);
    if (status != 0) {
        return status;
    }
    status = pjsua_call_answer(engine->call_id, PJSIP_SC_BUSY_HERE, NULL, NULL);
    return status == PJ_SUCCESS ? 0 : tp_fail(engine, status, "pjsua_call_answer(486)");
}

int tp_engine_hang_up(tp_engine *engine) {
    pj_status_t status;
    if (engine == NULL || engine->call_id == PJSUA_INVALID_ID) {
        return -1;
    }
    status = tp_ensure_thread_registered(engine);
    if (status != 0) {
        return status;
    }
    status = pjsua_call_hangup(engine->call_id, 0, NULL, NULL);
    return status == PJ_SUCCESS ? 0 : tp_fail(engine, status, "pjsua_call_hangup");
}

int tp_engine_send_dtmf(tp_engine *engine, char digit) {
    char digits[2] = {digit, '\0'};
    pj_str_t value = tp_pj_str(digits);
    pj_status_t status;
    if (engine == NULL || engine->call_id == PJSUA_INVALID_ID ||
        strchr("0123456789*#", digit) == NULL) {
        return -1;
    }
    status = tp_ensure_thread_registered(engine);
    if (status != 0) {
        return status;
    }
    status = pjsua_call_dial_dtmf(engine->call_id, &value);
    return status == PJ_SUCCESS ? 0 : tp_fail(engine, status, "pjsua_call_dial_dtmf");
}

int tp_engine_set_muted(tp_engine *engine, int muted) {
    pjsua_conf_port_id call_port;
    pj_status_t status;
    if (engine == NULL || engine->call_id == PJSUA_INVALID_ID) {
        return -1;
    }
    status = tp_ensure_thread_registered(engine);
    if (status != 0) {
        return status;
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
    return tp_fail(engine, status, "microphone mute");
}

const char *tp_last_error(const tp_engine *engine) {
    return engine != NULL ? engine->last_error : "PJSIP engine is not running";
}
