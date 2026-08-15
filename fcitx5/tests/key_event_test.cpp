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
  if (printableTarget.key_sym() != FcitxKey_a ||
      printableTarget.text() != "a" || printableTarget.input() != "a" ||
      printableTarget.intention() != "a") {
    std::cerr << "printable key mapping changed unexpectedly\n";
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
  return valid ? 0 : 1;
}
