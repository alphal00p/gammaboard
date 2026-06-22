ccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc
c      written by the UFO converter
ccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc

      SUBROUTINE COUP1( )
      USE MODEL_OBJECT
      IMPLICIT NONE

      INCLUDE 'model_functions.inc'

      DOUBLE PRECISION PI, ZERO
      PARAMETER  (PI=3.141592653589793D0)
      PARAMETER  (ZERO=0D0)
      INCLUDE 'input.inc'
      INCLUDE 'coupl.inc'
      GC_3 = -(MDL_EE*MDL_COMPLEXI)
      GC_FFV_2 = 2.000000D+00*((MDL_EE*MDL_COMPLEXI*MDL_SW)/(2.000000D
     $ +00*MDL_CW))
      GC_FFV_3 = 1.000000D+00*(-(MDL_CW*MDL_EE*MDL_COMPLEXI)
     $ /(2.000000D+00*MDL_SW)+((MDL_EE*MDL_COMPLEXI*MDL_SW)/(2.000000D
     $ +00*MDL_CW)))
      END
