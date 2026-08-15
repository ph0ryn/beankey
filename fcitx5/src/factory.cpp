#include "engine.h"

#include <fcitx/addonfactory.h>
#include <fcitx/addonmanager.h>

namespace fcitx {

class BeankeyEngineFactory final : public AddonFactory {
public:
  AddonInstance *create(AddonManager *manager) override {
    return new BeankeyEngine(manager->instance());
  }
};

} // namespace fcitx

#ifdef FCITX_ADDON_FACTORY_V2
FCITX_ADDON_FACTORY_V2(beankey, fcitx::BeankeyEngineFactory)
#else
FCITX_ADDON_FACTORY(fcitx::BeankeyEngineFactory)
#endif
