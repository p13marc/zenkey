// S15 (#617): a zenoh-pico 1.10.1 participant as a zk2 owner.
//
//   zk2_pico <locator> <key prefix> <service>
//
// The key prefix is "zk2" normally, or "<namespace>/zk2" for the namespace
// variant (zenoh-pico has no session namespace, so a deployment prefix is
// literal in its keys). It implements a minimal health.v1:
//   stream   <p>/vehicle-01/<svc>/health.v1/stream/heartbeat   (10 Hz)
//   state    <p>/vehicle-01/<svc>/health.v1/state/status       (S1-S3: stamped,
//            served by a queryable over state/**, deletes answered with reply_del)
//   ops      <p>/vehicle-01/<svc>/health.v1/@op/reset          (complete)
//            <p>/vehicle-01/<svc>/health.v1/@op/toggle         (delete or re-put the state)
//            <p>/vehicle-01/<svc>/health.v1/@op/big/<n>        (replies n bytes)
//   tokens   <p>/vehicle-01/<svc>/@zk/instance/<id>, …/@zk/alive/health.v1/<id>/<fp16>
//   descriptor: a queryable on the instance key
// Bring-up order (r3 §3.10): queryables, then the instance token, then the
// interface token. Prints "ready" on stdout.

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include <zenoh-pico.h>

static char status_key[256], state_sel[256], hb_key[256], reset_key[256], toggle_key[256], big_sel[256];
static char inst_key[256], alive_key[256], desc[512];
static int deleted = 0;
static z_timestamp_t state_ts;
static const z_loaned_session_t *S;
static z_owned_publisher_t hb;

static void stamp(void) { z_timestamp_new(&state_ts, S); }

static void put_status(void) {
    stamp();
    z_put_options_t o;
    z_put_options_default(&o);
    o.timestamp = &state_ts;
    z_view_keyexpr_t ke;
    z_view_keyexpr_from_str(&ke, status_key);
    z_owned_bytes_t b;
    z_bytes_copy_from_str(&b, "{\"state\":\"ok\"}");
    z_put(S, z_loan(ke), z_move(b), &o);
    deleted = 0;
}

static void delete_status(void) {
    stamp();
    z_delete_options_t o;
    z_delete_options_default(&o);
    o.timestamp = &state_ts;
    z_view_keyexpr_t ke;
    z_view_keyexpr_from_str(&ke, status_key);
    z_delete(S, z_loan(ke), &o);
    deleted = 1;
}

static void on_state(z_loaned_query_t *q, void *ctx) {
    (void)ctx;
    z_view_keyexpr_t ke;
    z_view_keyexpr_from_str(&ke, status_key);
    if (!z_keyexpr_intersects(z_query_keyexpr(q), z_loan(ke))) return;
    if (deleted) {
        z_query_reply_del_options_t o;
        z_query_reply_del_options_default(&o);
        o.timestamp = &state_ts;
        z_query_reply_del(q, z_loan(ke), &o);
    } else {
        z_query_reply_options_t o;
        z_query_reply_options_default(&o);
        o.timestamp = &state_ts;
        z_owned_bytes_t b;
        z_bytes_copy_from_str(&b, "{\"state\":\"ok\"}");
        z_query_reply(q, z_loan(ke), z_move(b), &o);
    }
}

static void on_reset(z_loaned_query_t *q, void *ctx) {
    (void)ctx;
    z_owned_bytes_t b;
    z_bytes_copy_from_str(&b, "ok");
    z_query_reply(q, z_query_keyexpr(q), z_move(b), NULL);
}

static void on_toggle(z_loaned_query_t *q, void *ctx) {
    (void)ctx;
    if (deleted) put_status(); else delete_status();
    z_owned_bytes_t b;
    z_bytes_copy_from_str(&b, deleted ? "deleted" : "put");
    z_query_reply(q, z_query_keyexpr(q), z_move(b), NULL);
}

static void on_big(z_loaned_query_t *q, void *ctx) {
    (void)ctx;
    z_view_string_t ks;
    z_keyexpr_as_view_string(z_query_keyexpr(q), &ks);
    const char *s = z_string_data(z_loan(ks));
    size_t len = z_string_len(z_loan(ks));
    const char *slash = NULL;
    for (size_t i = 0; i < len; i++) if (s[i] == '/') slash = s + i;
    size_t n = slash ? (size_t)strtoul(slash + 1, NULL, 10) : 0;
    unsigned char *buf = malloc(n ? n : 1);
    memset(buf, 'x', n);
    z_owned_bytes_t b;
    z_bytes_copy_from_buf(&b, buf, n);
    free(buf);
    z_query_reply(q, z_query_keyexpr(q), z_move(b), NULL);
}

static void on_desc(z_loaned_query_t *q, void *ctx) {
    (void)ctx;
    z_owned_bytes_t b;
    z_bytes_copy_from_str(&b, desc);
    z_query_reply(q, z_query_keyexpr(q), z_move(b), NULL);
}

static void queryable(const char *key, void (*cb)(z_loaned_query_t *, void *), int complete, z_owned_queryable_t *out) {
    z_view_keyexpr_t ke;
    z_view_keyexpr_from_str(&ke, key);
    z_owned_closure_query_t c;
    z_closure(&c, cb, NULL, NULL);
    z_queryable_options_t o;
    z_queryable_options_default(&o);
    o.complete = complete;
    if (z_declare_queryable(S, out, z_loan(ke), z_move(c), &o) < 0) {
        fprintf(stderr, "queryable %s failed\n", key);
        exit(1);
    }
}

int main(int argc, char **argv) {
    if (argc < 4) {
        fprintf(stderr, "usage: zk2_pico <locator> <key prefix> <service>\n");
        return 2;
    }
    const char *loc = argv[1], *p = argv[2], *svc = argv[3];
    char id[17];
    snprintf(id, sizeof id, "%016llx", (unsigned long long)time(NULL) * 1000003ULL);
    snprintf(status_key, sizeof status_key, "%s/vehicle-01/%s/health.v1/state/status", p, svc);
    snprintf(state_sel, sizeof state_sel, "%s/vehicle-01/%s/health.v1/state/**", p, svc);
    snprintf(hb_key, sizeof hb_key, "%s/vehicle-01/%s/health.v1/stream/heartbeat", p, svc);
    snprintf(reset_key, sizeof reset_key, "%s/vehicle-01/%s/health.v1/@op/reset", p, svc);
    snprintf(toggle_key, sizeof toggle_key, "%s/vehicle-01/%s/health.v1/@op/toggle", p, svc);
    snprintf(big_sel, sizeof big_sel, "%s/vehicle-01/%s/health.v1/@op/big/*", p, svc);
    snprintf(inst_key, sizeof inst_key, "%s/vehicle-01/%s/@zk/instance/%s", p, svc, id);
    snprintf(alive_key, sizeof alive_key, "%s/vehicle-01/%s/@zk/alive/health.v1/%s/0000000000000000", p, svc, id);
    snprintf(desc, sizeof desc,
             "{\"service\":\"vehicle-01/%s\",\"instance\":\"%s\",\"interfaces\":[{\"iface\":\"health.v1\"}],"
             "\"meta\":{\"build\":\"zenoh-pico 1.10.1\"}}",
             svc, id);

    z_owned_config_t config;
    z_config_default(&config);
    zp_config_insert(z_loan_mut(config), Z_CONFIG_MODE_KEY, "client");
    zp_config_insert(z_loan_mut(config), Z_CONFIG_CONNECT_KEY, loc);
    z_owned_session_t s;
    if (z_open(&s, z_move(config), NULL) < 0) {
        fprintf(stderr, "open failed\n");
        return 1;
    }
    zp_start_read_task(z_loan_mut(s), NULL);
    zp_start_lease_task(z_loan_mut(s), NULL);
    S = z_loan(s);

    // 1. Resources: the stream publisher, the state, the operations.
    z_view_keyexpr_t hk;
    z_view_keyexpr_from_str(&hk, hb_key);
    z_declare_publisher(S, &hb, z_loan(hk), NULL);
    put_status();
    z_owned_queryable_t q1, q2, q3, q4, q5;
    queryable(state_sel, on_state, 0, &q1);
    queryable(reset_key, on_reset, 1, &q2);
    queryable(toggle_key, on_toggle, 1, &q3);
    queryable(big_sel, on_big, 1, &q4);
    // 3. The descriptor; 4. the instance token, then the interface token.
    queryable(inst_key, on_desc, 1, &q5);
    z_owned_liveliness_token_t t1, t2;
    z_view_keyexpr_t ik, ak;
    z_view_keyexpr_from_str(&ik, inst_key);
    z_view_keyexpr_from_str(&ak, alive_key);
    z_liveliness_declare_token(S, &t1, z_loan(ik), NULL);
    z_liveliness_declare_token(S, &t2, z_loan(ak), NULL);
    printf("ready %s\n", id);
    fflush(stdout);

    unsigned long seq = 0;
    for (;;) {
        char msg[64];
        snprintf(msg, sizeof msg, "%lu", seq++);
        z_owned_bytes_t b;
        z_bytes_copy_from_str(&b, msg);
        z_publisher_put(z_loan(hb), z_move(b), NULL);
        z_sleep_ms(100);
    }
}
