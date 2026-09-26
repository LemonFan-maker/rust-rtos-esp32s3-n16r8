# 目标板运行记录（evidence/）

本目录保存ESP32-S3-N16R8开发板上采集的原始串口日志，用于说明OTA升级与自动回滚流程的实际运行结果。
日志由`cargo espflash monitor`抓取，未做任何编辑；板端固件由`partitions.csv`（16MB布局）烧录。

## 文件说明

| 文件 | 场景 | 可见结果 |
|------|------|----------|
| `rustrtos_ota_serial.log` | factory镜像启动后经HTTP下载505632字节镜像 | `OTA image written to Ota0 (505632 bytes); rebooting` → 复位后从`0x400000`（ota_0）加载；随后打印`No pending OTA confirmation`、`No OTA update available for version 0.2.0`，结尾出现`LoadProhibited`异常 |
| `rustrtos_ota_fixed_serial.log` | 同上场景，停止OTA任务但保留WiFi与协议栈任务 | 下载515296字节并写入ota_0，复位后从ota_0加载，结尾为`OTA task halted (keeping WiFi/net_task alive).`，无异常 |
| `rustrtos_rollback_serial.log` | 启用回滚引导程序的A/B切换验证 | `Confirmed the pending OTA image after WiFi/DHCP self-check` → 写入ota_1（92528字节）→ 从`0x800000`（ota_1）加载并以`OTA_ROLLBACK_PROBE_START_WITHOUT_CONFIRM`复位且不确认 → 下一次启动回到`0x400000`（ota_0） |
| `rustrtos_factory_safe_serial.log` | 引导程序与安全烧录检查 | 分区表与factory镜像加载正常；`WiFi connection failed`（当时接入的AP不可用，属环境因素） |

## 这些记录能说明什么

- OTA状态机（下载、写入非当前运行槽、切换启动槽、复位、从新槽启动）在上述日志中完整走通；
- 自动回滚依赖自行编译并启用`CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE=y`的第二阶段引导程序，`rustrtos_rollback_serial.log`中的bootloader编译时间为2026-09-25（ESP-IDF v6.1）；
- `rustrtos_ota_serial.log`记录的`LoadProhibited`异常对应OTA任务在镜像写完后自行退出、后续网络任务仍访问已释放资源的缺陷；修复方式见`examples/ota_update.rs`中保留WiFi与协议栈任务的处理，`rustrtos_ota_fixed_serial.log`为该处理后的记录。

## 这些记录不能说明什么

- 不构成性能指标：日志中的下载耗时、吞吐与时延仅反映当次网络与对端服务条件；
- 不构成安全保证：示例使用明文HTTP且仅校验ESP镜像起始magic字节，不含TLS、签名或来源认证；
- 不构成通用兼容性结论：以上结果仅对应ESP32-S3-N16R8模组、`partitions.csv`布局与上述bootloader配置；
- `rustrtos_ota_serial.log`中`No pending OTA confirmation`说明该次运行新镜像未处于待确认状态，因此该文件不用于证明回滚行为，回滚证据只取`rustrtos_rollback_serial.log`。

## 复现方式

```bash
# 1. 首次部署须显式指定分区表
cargo espflash flash --example ota_update --features network,dev \
    --partition-table partitions.csv --monitor

# 2. 准备HTTP服务提供镜像（可信隔离网络内）
#    设备端参数为examples/ota_update.rs中的WIFI_SSID/WIFI_PASSWORD/OTA_HOST/OTA_PORT/OTA_PATH占位符

# 3. 回滚验证需另编译启用CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE=y的第二阶段引导程序，
#    并注意重新执行cargo espflash flash可能覆盖0x0处的引导程序
```
