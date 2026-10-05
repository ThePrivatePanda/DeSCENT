/* Does sh2_devSleep() (BNO08x::modeSleep) ever return if an I2C write fails?
   Runs the library's real sh2.c/shtp.c with a write() that always fails like SparkFun's does.
   Not part of make test. Build it with:
   gcc imu_sleep_hang.c <lib>/src/sh2.c <lib>/src/shtp.c <lib>/src/sh2_util.c -I<lib>/src -o imu_sleep_hang */
#include <stdio.h>
#include <stdlib.h>
#include "sh2.h"
#include "sh2_hal.h"
static unsigned long writes=0; static uint32_t t=0;
static int h_open(sh2_Hal_t*s){return 0;}
static void h_close(sh2_Hal_t*s){}
static int h_read(sh2_Hal_t*s,uint8_t*b,unsigned n,uint32_t*tu){*tu=t;return 0;}
static int h_write(sh2_Hal_t*s,uint8_t*b,unsigned n){
  if(++writes>=5000000UL){printf("sh2_devSleep still inside HAL write retry loop after %lu failed writes -> never returns\n",writes);exit(3);}
  return 0;}
static uint32_t h_time(sh2_Hal_t*s){t+=1000;return t;}
static void ev(void*c,sh2_AsyncEvent_t*e){}
int main(void){
  sh2_Hal_t hal={h_open,h_close,h_read,h_write,h_time};
  int rc=sh2_open(&hal,ev,NULL); printf("sh2_open rc=%d (no reset seen, timed out)\n",rc);
  rc=sh2_devSleep(); printf("sh2_devSleep returned %d\n",rc); return 0;}
