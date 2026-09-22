#define _GNU_SOURCE 1

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <locale.h>
#include <time.h>
#include <unistd.h>
#include <wchar.h>

#include "builtins.h"
#include "quit.h"
#include <readline/readline.h>

/* Display-only Bash builtin. The preview is never put in rl_line_buffer. */
static char *prefix;
static char *preview;
static double expires;
static int painted;
static int redisplaying;
static int displayed;
static locale_t utf8_locale;
static rl_voidfunc_t *previous_redisplay;
static rl_voidfunc_t *previous_deprep;
static rl_hook_func_t *previous_event;
static const char *preview_style;
static const char *preview_reset;

static void choose_preview_style(void) {
    const char *color = getenv("COLORTERM");
    const char *terminal = getenv("TERM");
    preview_reset = "\033[39m";
    /* Palette slot 8 can be almost the background color in a dark theme.
     * Neutral mid-grey stays distinct on both dark and light backgrounds,
     * without querying the terminal or consuming Readline's input stream. */
    if (color && (!strcmp(color, "truecolor") || !strcmp(color, "24bit"))) {
        preview_style = "\033[38;2;128;128;128m";
    } else if (terminal && strstr(terminal, "256color")) {
        preview_style = "\033[38;5;244m";
    } else {
        /* Basic terminals retain the user's readable foreground; request
         * italic differentiation instead of guessing a contrasting palette. */
        preview_style = "\033[39;3m";
        preview_reset = "\033[23;39m";
    }
}

static double now_seconds(void) {
    struct timespec now;
    if (clock_gettime(CLOCK_MONOTONIC, &now)) return -1;
    return (double)now.tv_sec + (double)now.tv_nsec / 1000000000.0;
}

static void forget(void) {
    free(prefix); prefix = NULL;
    free(preview); preview = NULL;
    displayed = 0;
}

/* Count the last prompt line; bracketed terminal control sequences have zero
 * width according to Readline's public prompt contract. Reject other controls. */
static int columns(const char *text, int prompt) {
    mbstate_t state = {0};
    int width = 0, ignored = 0;
    while (text && *text) {
        unsigned char byte = (unsigned char)*text;
        if (prompt && byte == RL_PROMPT_START_IGNORE) { ignored = 1; text++; continue; }
        if (prompt && byte == RL_PROMPT_END_IGNORE) { ignored = 0; text++; continue; }
        if (ignored) { text++; continue; }
        if (prompt && (*text == '\n' || *text == '\r')) { width = 0; text++; continue; }
        if (byte < 32 || byte == 127) return -1;
        wchar_t character;
        size_t size = mbrtowc(&character, text, MB_CUR_MAX, &state);
        if (size == (size_t)-1 || size == (size_t)-2 || !size) return -1;
        int cells = wcwidth(character);
        if (cells < 0 || width > 16384 - cells) return -1;
        width += cells;
        text += size;
    }
    return ignored ? -1 : width;
}

static void hide(void) {
    if (!painted) return;
    painted = 0;
    /* Readline owns the old cursor and wrapped-line geometry. Clear through
     * its API, then force its own renderer to reconstruct the real command. */
    rl_clear_visible_line();
    redisplaying = 1;
    rl_redraw_prompt_last_line();
    redisplaying = 0;
}

static int current(void) {
    double now = now_seconds();
    return preview && prefix && !rl_done && rl_point == rl_end && rl_line_buffer &&
        strcmp(prefix, rl_line_buffer) == 0 && now >= 0 && now < expires &&
        !(rl_readline_state & (RL_STATE_ISEARCH | RL_STATE_NSEARCH | RL_STATE_COMPLETING));
}

static int fits(const char *before, const char *text) {
    locale_t previous_locale = uselocale(utf8_locale);
    int prompt_width = columns(rl_prompt, 1);
    int line_width = columns(before, 0);
    int preview_width = columns(text, 0);
    uselocale(previous_locale);
    int rows, width;
    rl_get_screen_size(&rows, &width);
    (void)rows;
    return prompt_width >= 0 && line_width >= 0 && preview_width > 0 &&
        prompt_width + line_width + preview_width < width;
}

static void redisplay(void) {
    if (redisplaying) { previous_redisplay(); return; }
    hide();
    previous_redisplay();
    if (!current()) { forget(); return; }
    /* Keep one spare terminal cell: drawing must never trigger wrap/scroll.
     * Complex, wrapped and multiline commands retain ordinary Readline. */
    if (!fits(rl_line_buffer, preview)) { forget(); return; }
    fprintf(rl_outstream, "\0337%s%s%s\0338", preview_style, preview, preview_reset);
    fflush(rl_outstream);
    painted = 1;
    displayed = 1;
}

static int event(void) {
    /* Readline's timed event loop bypasses Bash's blocking input callback.
     * Preserve the shell's documented deferred interrupt/termination checks. */
    if (interrupt_state || terminating_signal) { hide(); forget(); }
    QUIT;
    int result = previous_event ? previous_event() : 0;
    if (preview && !current()) { hide(); forget(); }
    return result;
}

static void deprep(void) {
    hide();
    if (rl_done) forget();
    if (previous_deprep) previous_deprep();
}

static int badi_preview_builtin(WORD_LIST *list) {
    if (!list) return 2;
    const char *operation = list->word->word;
    list = list->next;
    if (strcmp(operation, "clear") == 0 && !list) { hide(); forget(); return 0; }
    if (strcmp(operation, "active") == 0 && list && !list->next) {
        return displayed && current() && strcmp(prefix, list->word->word) == 0 ? 0 : 1;
    }
    if (strcmp(operation, "eligible") == 0 && list && !list->next) {
        return rl_point == rl_end && strlen(list->word->word) <= 8192 ? 0 : 1;
    }
    if (strcmp(operation, "show") || !list || !list->next || list->next->next) return 2;
    const char *text = list->word->word;
    const char *before = list->next->word->word;
    double now = now_seconds();
    if (strlen(text) > 256 || strlen(before) > 8192 || !*text || !*before || !fits(before, text)) return 2;
    if (now < 0) return 1;
    hide(); forget();
    preview = strdup(text);
    prefix = strdup(before);
    if (!preview || !prefix) { forget(); return 1; }
    expires = now + 4.0;
    return 0;
}

int badi_preview_builtin_load(char *name) {
    (void)name;
    if (!isatty(STDIN_FILENO) || !isatty(STDOUT_FILENO) || rl_redisplay_function != rl_redisplay) return 0;
    /* Readline skips terminal discovery for a custom renderer. Initialize its
     * normal terminal capabilities before installing this small overlay. */
    if (!(rl_readline_state & RL_STATE_INITIALIZED)) rl_initialize();
    if (!rl_get_termcap("ce")) return 0;
    utf8_locale = newlocale(LC_CTYPE_MASK, "C.UTF-8", (locale_t)0);
    if (!utf8_locale) return 0;
    choose_preview_style();
    previous_redisplay = rl_redisplay_function;
    previous_deprep = rl_deprep_term_function;
    previous_event = rl_event_hook;
    rl_redisplay_function = redisplay;
    rl_deprep_term_function = deprep;
    rl_event_hook = event;
    return 1;
}

void badi_preview_builtin_unload(char *name) {
    (void)name;
    hide(); forget();
    if (rl_redisplay_function == redisplay) rl_redisplay_function = previous_redisplay;
    if (rl_deprep_term_function == deprep) rl_deprep_term_function = previous_deprep;
    if (rl_event_hook == event) rl_event_hook = previous_event;
    freelocale(utf8_locale);
}

static char *const documentation[] = {
    "Render a temporary grey Badi preview without changing the Readline buffer.",
    "show TEXT PREFIX displays only at an unchanged command's end; clear dismisses.",
    "active PREFIX checks a displayed preview; eligible PREFIX checks the caret and byte limit.",
    NULL,
};

struct builtin badi_preview_struct = {
    "badi_preview", badi_preview_builtin, BUILTIN_ENABLED, documentation,
    "badi_preview show TEXT PREFIX | active PREFIX | eligible PREFIX | clear", NULL,
};
