#include "key_event.h"

#include <fcitx-utils/key.h>
#include <fcitx-utils/keysym.h>
#include <fcitx/event.h>

#include <array>
#include <cstdint>
#include <iostream>
#include <string_view>

#include "beankey.pb.h"

namespace {

struct KeyCase {
  std::string_view name;
  fcitx::KeySym symbol;
};

bool rejectsControlText(const KeyCase &keyCase) {
  fcitx::KeyEvent source(nullptr, fcitx::Key(keyCase.symbol));
  beankey::v1::KeyEvent target;
  beankey::populateKeyEvent(source, &target);

  if (target.text().empty() && target.input().empty() &&
      target.intention().empty()) {
    return true;
  }
  std::cerr << keyCase.name << " leaked control text into the protocol\n";
  return false;
}

} // namespace

int main() {
  fcitx::KeyEvent printableSource(nullptr, fcitx::Key(FcitxKey_a));
  beankey::v1::KeyEvent printableTarget;
  beankey::populateKeyEvent(printableSource, &printableTarget);
  if (printableTarget.action() != beankey::v1::USER_ACTION_INPUT ||
      printableTarget.text() != "a" || printableTarget.input() != "a" ||
      printableTarget.intention() != "a") {
    std::cerr << "printable key mapping changed unexpectedly\n";
    return 1;
  }

  fcitx::KeyEvent longVowelSource(nullptr, fcitx::Key(FcitxKey_minus));
  beankey::v1::KeyEvent longVowelTarget;
  beankey::populateKeyEvent(longVowelSource, &longVowelTarget);
  if (longVowelTarget.action() != beankey::v1::USER_ACTION_INPUT ||
      longVowelTarget.text() != "ー" || longVowelTarget.input() != "-" ||
      longVowelTarget.intention() != "ー") {
    std::cerr << "Japanese hyphen key did not produce a long vowel mark\n";
    return 1;
  }

  constexpr std::array controlKeys{
      KeyCase{"BackSpace", FcitxKey_BackSpace},
      KeyCase{"Return", FcitxKey_Return},
      KeyCase{"KP_Enter", FcitxKey_KP_Enter},
      KeyCase{"Delete", FcitxKey_Delete},
      KeyCase{"Escape", FcitxKey_Escape},
      KeyCase{"Tab", FcitxKey_Tab},
      KeyCase{"ISO_Left_Tab", FcitxKey_ISO_Left_Tab},
  };
  bool valid = true;
  for (const auto &keyCase : controlKeys) {
    valid = rejectsControlText(keyCase) && valid;
  }
  fcitx::KeyEvent backspaceSource(nullptr, fcitx::Key(FcitxKey_BackSpace));
  beankey::v1::KeyEvent backspaceTarget;
  beankey::populateKeyEvent(backspaceSource, &backspaceTarget);
  if (backspaceTarget.action() != beankey::v1::USER_ACTION_BACKSPACE) {
    std::cerr << "Backspace semantic action was not mapped\n";
    valid = false;
  }

  fcitx::KeyEvent controlHSource(nullptr,
                                 fcitx::Key(FcitxKey_h, fcitx::KeyState::Ctrl));
  beankey::v1::KeyEvent controlHTarget;
  beankey::populateKeyEvent(controlHSource, &controlHTarget);
  if (controlHTarget.action() != beankey::v1::USER_ACTION_BACKSPACE ||
      !controlHTarget.text().empty()) {
    std::cerr << "Control-H semantic action was not mapped\n";
    valid = false;
  }

  fcitx::KeyEvent shiftedSpaceSource(
      nullptr, fcitx::Key(FcitxKey_space, fcitx::KeyState::Shift));
  beankey::v1::KeyEvent shiftedSpaceTarget;
  beankey::populateKeyEvent(shiftedSpaceSource, &shiftedSpaceTarget);
  if (shiftedSpaceTarget.action() != beankey::v1::USER_ACTION_SPACE ||
      !shiftedSpaceTarget.shift()) {
    std::cerr << "Shift-Space semantic action was not mapped\n";
    valid = false;
  }
  return valid ? 0 : 1;
}
