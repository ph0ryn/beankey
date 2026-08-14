#include <llama.h>
#include <ggml-backend.h>

#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

struct beankey_llama {
    struct llama_model *model;
    struct llama_context *context;
    const struct llama_vocab *vocab;
    struct llama_batch batch;
};

static void set_error(char *error, size_t error_capacity, const char *message) {
    if (error == NULL || error_capacity == 0) {
        return;
    }
    const size_t length = strlen(message);
    const size_t copy_length = length < error_capacity - 1 ? length : error_capacity - 1;
    memcpy(error, message, copy_length);
    error[copy_length] = '\0';
}

struct beankey_llama *beankey_llama_load(
    const char *path,
    const char *backend_directory,
    int32_t thread_count,
    char *error,
    size_t error_capacity
) {
    ggml_backend_load_all_from_path(backend_directory);
    llama_backend_init();
    struct llama_model_params model_params = llama_model_default_params();
    struct llama_model_kv_override overrides[2] = {0};
    overrides[0].tag = LLAMA_KV_OVERRIDE_TYPE_STR;
    strcpy(overrides[0].key, "tokenizer.ggml.pre");
    strcpy(overrides[0].val_str, "gpt-2");
    model_params.kv_overrides = overrides;
    struct llama_model *model = llama_model_load_from_file(path, model_params);
    if (model == NULL) {
        set_error(error, error_capacity, "llama.cpp could not load the model");
        return NULL;
    }

    struct llama_context_params context_params = llama_context_default_params();
    context_params.n_ctx = 512;
    context_params.n_batch = 512;
    context_params.n_ubatch = 64;
    context_params.n_seq_max = 1;
    context_params.flash_attn_type = LLAMA_FLASH_ATTN_TYPE_ENABLED;
    context_params.n_threads = thread_count;
    context_params.n_threads_batch = thread_count;
    struct llama_context *context = llama_init_from_model(model, context_params);
    if (context == NULL) {
        llama_model_free(model);
        set_error(error, error_capacity, "llama.cpp could not create the inference context");
        return NULL;
    }

    struct beankey_llama *handle = calloc(1, sizeof(*handle));
    if (handle == NULL) {
        llama_free(context);
        llama_model_free(model);
        set_error(error, error_capacity, "memory allocation failed");
        return NULL;
    }
    handle->model = model;
    handle->context = context;
    handle->vocab = llama_model_get_vocab(model);
    handle->batch = llama_batch_init(512, 0, 1);
    return handle;
}

void beankey_llama_free(struct beankey_llama *handle) {
    if (handle == NULL) {
        return;
    }
    llama_batch_free(handle->batch);
    llama_free(handle->context);
    llama_model_free(handle->model);
    free(handle);
}

int32_t beankey_llama_vocab_size(const struct beankey_llama *handle) {
    return llama_vocab_n_tokens(handle->vocab);
}

int32_t beankey_llama_eos_token(const struct beankey_llama *handle) {
    return llama_vocab_eos(handle->vocab);
}

int32_t beankey_llama_tokenize(
    const struct beankey_llama *handle,
    const char *text,
    int32_t text_length,
    int32_t *tokens,
    int32_t token_capacity,
    bool add_special
) {
    return llama_tokenize(
        handle->vocab,
        text,
        text_length,
        tokens,
        token_capacity,
        add_special,
        false
    );
}

int32_t beankey_llama_token_to_piece(
    const struct beankey_llama *handle,
    int32_t token,
    char *buffer,
    int32_t buffer_capacity
) {
    return llama_token_to_piece(handle->vocab, token, buffer, buffer_capacity, 0, false);
}

int32_t beankey_llama_next_logits(
    struct beankey_llama *handle,
    const int32_t *tokens,
    int32_t token_count,
    float *logits,
    int32_t logits_capacity
) {
    const int32_t vocabulary_size = llama_vocab_n_tokens(handle->vocab);
    if (token_count <= 0 || token_count > 512 || logits_capacity < vocabulary_size) {
        return -1;
    }
    llama_memory_clear(llama_get_memory(handle->context), true);
    handle->batch.n_tokens = token_count;
    for (int32_t index = 0; index < token_count; ++index) {
        handle->batch.token[index] = tokens[index];
        handle->batch.pos[index] = index;
        handle->batch.n_seq_id[index] = 1;
        handle->batch.seq_id[index][0] = 0;
        handle->batch.logits[index] = index == token_count - 1 ? 1 : 0;
    }
    const int32_t decode_result = llama_decode(handle->context, handle->batch);
    if (decode_result != 0) {
        return decode_result;
    }
    const float *model_logits = llama_get_logits_ith(handle->context, -1);
    if (model_logits == NULL) {
        return -2;
    }
    memcpy(logits, model_logits, (size_t)vocabulary_size * sizeof(*logits));
    return 0;
}
