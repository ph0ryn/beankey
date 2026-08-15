#include "key_event.h"

#include <fcitx-utils/key.h>
#include <fcitx/event.h>

#include <cstdint>
#include <string>

#include "beankey.pb.h"

namespace beankey {
namespace {

constexpr std::uint32_t kShiftModifier = 1;

std::string printableKeyText(fcitx::KeySym symbol) {
  const std::uint32_t codePoint = fcitx::Key::keySymToUnicode(symbol);
  if (codePoint < 0x20 || (codePoint >= 0x7f && codePoint <= 0x9f)) {
    return {};
  }
  return fcitx::Key::keySymToUTF8(symbol);
}

} // namespace

void populateKeyEvent(const fcitx::KeyEvent &source, v1::KeyEvent *target) {
  target->set_key_sym(source.rawKey().sym());
  target->set_release(source.isRelease());
  if (source.rawKey().states().test(fcitx::KeyState::Shift)) {
    target->set_modifiers(kShiftModifier);
  }
  const fcitx::KeyStates shortcutModifiers(
      {fcitx::KeyState::Ctrl, fcitx::KeyState::Alt, fcitx::KeyState::Super,
       fcitx::KeyState::Super2, fcitx::KeyState::Hyper, fcitx::KeyState::Meta});
  if (!source.key().states().testAny(shortcutModifiers)) {
    target->set_text(printableKeyText(source.key().sym()));
  }
  const std::string rawText = printableKeyText(source.rawKey().sym());
  target->set_input(target->text().empty() ? rawText : target->text());
  target->set_intention(rawText);
}

} // namespace beankey
