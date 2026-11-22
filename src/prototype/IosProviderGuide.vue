<script setup lang="ts">
// Prototype of step 1's provider part: three neutral choices, then the numbered steps of the chosen
// service with copy chips for the strings that have to be typed there and a notice for what to watch.
// Facts mirror docs/wiki/ios-broker.md; no service is recommended (docs/specs/ios-open-access.md OA3).
import { computed } from "vue";
import { EMQX_NOTICE, PROVIDERS, type Provider, type SetupState } from "./iosSetupState";

// `folded`: the broker already works, so the instructions are tucked into a disclosure.
const props = defineProps<{ s: SetupState; folded?: boolean }>();

const ids = Object.keys(PROVIDERS) as Provider[];
const acl = computed(() => (props.s.username ? `ah/${props.s.username}/#` : ""));

function choose(id: Provider) {
  props.s.provider = id;
  if (id === "flespi") {
    props.s.host = "mqtt.flespi.io";
    props.s.wsPort = 443;
  } else if (id === "emqx") {
    props.s.wsPort = 8084;
  }
}

let timer: number | undefined;
function copy(text: string) {
  void navigator.clipboard?.writeText(text).catch(() => {});
  props.s.copied = text;
  window.clearTimeout(timer);
  timer = window.setTimeout(() => (props.s.copied = ""), 1500);
}
</script>

<template>
  <component :is="folded ? 'details' : 'div'" :class="{ 'ios2-more': folded, 'ios2-guide-fold': folded }">
  <summary v-if="folded">配置步骤{{ s.provider ? `（${PROVIDERS[s.provider].name}）` : "" }}</summary>
  <p class="ios2-lead">Broker 在 Mac 和手机之间转发消息，选一个服务商：</p>
  <div class="ios2-tiles" role="radiogroup" aria-label="Broker 服务商">
    <button
      v-for="id in ids"
      :key="id"
      type="button"
      class="ios2-tile"
      :class="{ active: s.provider === id }"
      role="radio"
      :aria-checked="s.provider === id"
      :data-testid="`ios-broker-${id}`"
      @click="choose(id)"
    >
      <b>{{ PROVIDERS[id].name }}</b>
      <span>{{ PROVIDERS[id].sub }}</span>
    </button>
  </div>

  <template v-if="s.provider === 'emqx'">
    <!-- First, before anyone registers: what happens when the free quota runs out. -->
    <p class="ios2-notice" data-testid="ios-emqx-notice">{{ EMQX_NOTICE }}</p>
    <ol class="ios2-list">
      <li>
        <a class="link" href="#" @click.prevent>打开 EMQX Cloud ↗</a>，注册并新建 <b>Serverless</b> 部署，
        <b>消费限额保持 0</b>。
      </li>
      <li>把部署概览里的「连接地址」填到下面的「主机」。</li>
      <li>「访问控制 → 客户端认证」：添加用户名和密码（用户名只用字母和数字），填到下面。</li>
      <li>
        「访问控制 → 客户端授权」：默认放行所有话题，<b>必须</b>按下面设置。
        <ul>
          <li>
            「全部用户」标签：话题
            <button type="button" class="ios2-chip" @click="copy('#')">
              #<span v-if="s.copied === '#'" class="ios2-copied">已复制</span>
            </button>
            ，「发布 &amp; 订阅」，选<b>不允许</b>。
          </li>
          <li>
            「用户名」标签：话题
            <button type="button" class="ios2-chip" :disabled="!acl" @click="copy(acl)">
              {{ acl || "ah/<用户名>/#" }}<span v-if="acl && s.copied === acl" class="ios2-copied">已复制</span>
            </button>
            ，「发布 &amp; 订阅」，选<b>允许</b>。
          </li>
        </ul>
      </li>
    </ol>
  </template>

  <template v-else-if="s.provider === 'flespi'">
    <p class="ios2-notice info">
      需要注意：免费套餐面向测试与开发；服务器在欧洲，大陆访问可能有延迟；账号 60 天没有活动会被删除。
    </p>
    <ol class="ios2-list">
      <li><a class="link" href="#" @click.prevent>打开 flespi ↗</a>，注册并保持 <b>Free</b> 套餐。</li>
      <li>
        「Tokens」→ 新建 token：类型选 <b>ACL</b>，有效期（TTL）填最大值 8760 小时；ACL 只留一条：
        <code>mqtt</code> 模块，publish 和 subscribe，主题
        <button type="button" class="ios2-chip" @click="copy('ah/#')">
          ah/#<span v-if="s.copied === 'ah/#'" class="ios2-copied">已复制</span></button
        >。
      </li>
      <li>
        记下 token 左上角 <code>#</code> 后面的<b>编号</b>，打开这个 token，把 ACL 主题改成
        <button type="button" class="ios2-chip" :disabled="!acl" @click="copy(acl)">
          {{ acl || "ah/<编号>/#" }}<span v-if="acl && s.copied === acl" class="ios2-copied">已复制</span></button
        >。
      </li>
      <li>「用户名」填<b>编号</b>，「密码」填<b>完整的 token</b>（只有新建时能复制）。</li>
    </ol>
  </template>

  <template v-else-if="s.provider === 'custom'">
    <ul class="ios2-list">
      <li>MQTT 3.1.1，QoS 1，保留消息和遗嘱，单条消息至少约 1 MB；用户名只含字母和数字。</li>
      <li>TLS 证书由公共 CA 签发（App 不接受自签），默认端口 8883；WSS 可选，路径 <code>/mqtt</code>，默认 8084。</li>
      <li>
        每个账号只能读写 <code>ah/&lt;用户名&gt;/#</code>（Mosquitto：
        <button type="button" class="ios2-chip" @click="copy('pattern readwrite ah/%u/#')">
          pattern readwrite ah/%u/#<span v-if="s.copied === 'pattern readwrite ah/%u/#'" class="ios2-copied">已复制</span></button
        >）。
      </li>
    </ul>
    <p class="ios2-lead">
      <a class="link" href="#" @click.prevent>Mosquitto 参考配置 ↗</a> ·
      <a class="link" href="#" @click.prevent>兼容套件 ↗</a>
    </p>
  </template>
  </component>
</template>
